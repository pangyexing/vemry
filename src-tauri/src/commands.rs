//! All IPC commands — the contract between the frontend and the Rust core.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

use vemry_agent::runner::{AgentEvent, AgentRunner, RunnerConfig, WritePolicy};
use vemry_agent::session::{
    delete_session, fork_session, read_history, read_session, rename_session, write_history,
    SessionLog, SessionMeta,
};
use vemry_agent::{HttpTransport, PermissionDecision};
use vemry_core::{
    BacklinkEntry, BlockHit, BlockIdPlan, BlockProvenance, BlockQuery, BlockSelector, GraphData, InsertLanded, InsertOrigin, MigrationReport, NoteSummary, ProvenanceEntry, RefResolution, RevertPlan, SearchHit, Vault, VaultEvent,
};

use crate::agent_bridge::{resolve, ChannelGate};
use crate::settings::{self, Settings};
use crate::AppState;

#[derive(Serialize)]
pub struct VaultInfo {
    pub root: String,
    pub note_count: usize,
}

/// The one place vault events fan out from — the concrete form of the
/// event.rs bus. The frontend always hears the event; `ScriptsChanged`
/// additionally hot-reloads the script host. Workflow dispatch (v0.3 M3)
/// hooks in here too, which is exactly why script-initiated writes must NOT
/// come through this function (they emit via the bridge instead — loop
/// prevention by construction).
pub(crate) fn broadcast(app: &AppHandle, event: &VaultEvent) {
    let host = {
        let state: State<'_, AppState> = app.state();
        let guard = state.scripts.lock().unwrap();
        guard.clone()
    };
    if let Some(host) = host {
        match event {
            VaultEvent::ScriptsChanged => host.reload(),
            // workflow triggers: user IPC writes + watcher external changes.
            // Script-initiated writes never reach broadcast, so they can't
            // re-trigger workflows.
            VaultEvent::NoteAdded { .. }
            | VaultEvent::NoteModified { .. }
            | VaultEvent::NoteRemoved { .. }
            | VaultEvent::FilesChanged => host.dispatch_event(event),
            _ => {}
        }
    }
    // The block types are snapshotted into the system prompt like the skills: a
    // change under `.vemry/types/` applies from the next turn
    if matches!(event, VaultEvent::TypesChanged) {
        let state: State<'_, AppState> = app.state();
        state.agent.invalidate_prompt_snapshot();
    }
    // VEMRY.md is snapshotted into the system prompt per session; a change to
    // it (a private block added, a convention edited) applies from the next
    // turn instead of the next session
    if let VaultEvent::NoteAdded { id } | VaultEvent::NoteModified { id } = event {
        if id.as_str() == "VEMRY.md" {
            let state: State<'_, AppState> = app.state();
            state.agent.invalidate_prompt_snapshot();
        }
    }
    let _ = app.emit("vault://event", event);
}

/// The vault that is open (None = none). The second webview (the detached chat
/// window) uses this at startup to pick up the vault the main window already has
/// open, rather than going through vault_open — that would restart the watcher and
/// switch vaults.
#[tauri::command]
pub fn vault_current(state: State<'_, AppState>) -> Option<VaultInfo> {
    let guard = state.vault.lock().unwrap();
    guard.as_ref().map(|v| VaultInfo { root: v.root().to_string_lossy().to_string(), note_count: v.index().note_count() })
}

pub(crate) fn with_vault<T>(state: &State<'_, AppState>, f: impl FnOnce(&Vault) -> Result<T, String>) -> Result<T, String> {
    let guard = state.vault.lock().unwrap();
    let vault = guard.as_ref().ok_or("no vault open")?;
    f(vault)
}

pub(crate) fn with_vault_mut<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&mut Vault) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.vault.lock().unwrap();
    let vault = guard.as_mut().ok_or("no vault open")?;
    f(vault)
}

#[tauri::command]
pub fn vault_open(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<VaultInfo, String> {
    let vault = Vault::open(&path).map_err(crate::i18n::err)?;
    let root = vault.root().to_string_lossy().to_string();
    let note_count = vault.index().note_count();

    // Switching vaults invalidates any in-flight agent turn: cancel it and
    // wake pending permission dialogs so the loop observes the flag before it
    // can write into the new vault.
    if state.agent.running.load(Ordering::SeqCst) {
        state.agent.cancel.store(true, Ordering::SeqCst);
        state.agent.pending.deny_all();
    }

    *state.vault.lock().unwrap() = Some(vault);

    // watcher: external changes -> incremental reindex -> event to frontend
    let vault_arc = Arc::clone(&state.vault);
    let app_handle = app.clone();
    let watcher = vemry_core::watcher::watch(std::path::Path::new(&root), move |paths| {
        let events = {
            let mut guard = vault_arc.lock().unwrap();
            match guard.as_mut() {
                Some(v) => v.handle_fs_change(&paths),
                None => vec![],
            }
        };
        for ev in events {
            broadcast(&app_handle, &ev);
        }
    })
    .map_err(crate::i18n::err)?;
    *state.watcher.lock().unwrap() = Some(watcher);

    // script host: spawned lazily on the first vault open (the UI bridge
    // needs an AppHandle), then reloaded on every switch. Pending grant or
    // prompt dialogs belong to the previous vault — wake them as denied.
    {
        state.script_bridge.deny_all();
        let mut scripts = state.scripts.lock().unwrap();
        if scripts.is_none() {
            let bridge = crate::script_bridge::TauriBridge {
                app: app.clone(),
                state: Arc::clone(&state.script_bridge),
            };
            *scripts = Some(vemry_script::ScriptHost::spawn(
                Arc::clone(&state.vault),
                Arc::new(bridge),
                vemry_script::Perspective::default(), // the user's perspective: a script has the same rights as the UI
                vemry_script::HostConfig::default(),
            ));
        }
        scripts.as_ref().unwrap().reload();
    }

    // A directory opened by mistake does not enter the recent history (recent/last):
    // it has to have notes or a .vemry/ to "look like a vault"; an empty directory
    // counts too — starting a new vault in an empty folder is an explicit intent.
    // Failing the check still opens it, it is just not remembered, and a restart
    // returns to the last real vault.
    if note_count > 0 || looks_like_vault(std::path::Path::new(&root)) {
        settings::remember_vault(&app, &root);
    }
    broadcast(&app, &VaultEvent::IndexReady { note_count });
    Ok(VaultInfo { root, note_count })
}

/// "Looks like a vault": it has a `.vemry/` config directory, or (hidden entries
/// aside) it is an empty directory.
/// The "has .md files" branch is decided by the caller from note_count — the index
/// has counted them already, so the disk is not touched again.
fn looks_like_vault(root: &std::path::Path) -> bool {
    root.join(".vemry").is_dir() || dir_effectively_empty(root)
}

/// The directory holds no non-hidden entry. System clutter does not count — Windows
/// hides files by attribute rather than a leading dot, and an "empty" folder synced
/// by a cloud drive often carries a desktop.ini. Unreadable counts as "not empty".
fn dir_effectively_empty(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries.filter_map(|e| e.ok()).all(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with('.')
                    || name.eq_ignore_ascii_case("desktop.ini")
                    || name.eq_ignore_ascii_case("thumbs.db")
            })
        })
        .unwrap_or(false)
}

/// Record the currently open vault in the recent history. The entry check runs once
/// in vault_open, and a directory holding nothing but pdf/txt is judged "not like a
/// vault" — the moment the first note is created it really is one, and this covers
/// that other moment. Already at the front, it is skipped, so creating a note does
/// not rewrite the settings file every time.
fn remember_open_vault(app: &AppHandle, state: &State<'_, AppState>) {
    let Ok(root) = with_vault(state, |v| Ok(v.root().to_string_lossy().to_string())) else {
        return;
    };
    if settings::load(app).recent_vaults.first() == Some(&root) {
        return;
    }
    settings::remember_vault(app, &root);
}

/// Create a vault: when the target does not exist (or holds nothing but hidden or
/// system clutter), create the directory, the `.vemry/` skeleton and the full
/// VEMRY.md template (the same wording as the ⌘P initialisation command), then go
/// through the same opening path as vault_open (watcher, script host and recent
/// history all identical — having written VEMRY.md, the recent-history check passes
/// naturally). A directory that already holds data is refused outright: "create"
/// must not silently become "open", or a user who picked a real directory by
/// mistake would think the vault was empty.
#[tauri::command]
pub fn vault_create(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<VaultInfo, String> {
    let p = std::path::Path::new(&path);
    if p.exists() {
        if !p.is_dir() {
            return Err(crate::i18n::tf(crate::i18n::current(), "ipc.fileExists", &[("path", &path)]));
        }
        if !dir_effectively_empty(p) {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.dirNotEmpty"));
        }
    }
    std::fs::create_dir_all(p.join(".vemry")).map_err(|e| e.to_string())?;
    // The emptiness check guarantees this overwrites no existing VEMRY.md (its name is neither hidden nor clutter)
    std::fs::write(p.join("VEMRY.md"), memory_template(&app)).map_err(|e| e.to_string())?;
    vault_open(app, state, path)
}

#[tauri::command]
pub fn vault_recent(app: AppHandle) -> Vec<String> {
    settings::load(&app).recent_vaults
}

/// Remove one entry from the recent list (a settings-level operation that does not
/// touch the directory on disk); returns the updated list.
#[tauri::command]
pub fn vault_recent_remove(app: AppHandle, path: String) -> Result<Vec<String>, String> {
    settings::forget_vault(&app, &path)
}

#[tauri::command]
pub fn vault_last(app: AppHandle) -> Option<String> {
    settings::load(&app).last_vault
}

#[tauri::command]
pub fn notes_list(state: State<'_, AppState>) -> Result<Vec<NoteSummary>, String> {
    with_vault(&state, |v| Ok(v.index().summaries()))
}

#[derive(Serialize)]
pub struct NoteContent {
    pub id: String,
    pub content: String,
}

/// Attachment-bucket migration (the old attachments/<stem>/ →
/// attachments/<note path minus .md>/).
/// A one-off **explicit** command (through ⌘P) that never runs by itself; dry_run
/// produces the report for the confirmation dialog.
/// A real run rescans once and broadcasts IndexReady to refresh the frontend — the
/// watcher echo of the directory moves is merged by the debounce, and at worst
/// there is one extra FilesChanged, which is harmless.
#[tauri::command]
pub fn attachments_migrate(
    app: AppHandle,
    state: State<'_, AppState>,
    dry_run: bool,
) -> Result<MigrationReport, String> {
    let report = with_vault_mut(&state, |v| v.migrate_attachment_buckets(dry_run).map_err(crate::i18n::err))?;
    if !dry_run {
        let note_count = with_vault_mut(&state, |v| {
            v.rescan().map_err(crate::i18n::err)?;
            Ok(v.index().note_count())
        })?;
        broadcast(&app, &VaultEvent::IndexReady { note_count });
    }
    Ok(report)
}

#[tauri::command]
pub fn note_read(state: State<'_, AppState>, path: String) -> Result<NoteContent, String> {
    with_vault(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        let content = v.read_note(&id).map_err(crate::i18n::err)?;
        Ok(NoteContent { id: id.to_string(), content })
    })
}

#[tauri::command]
pub fn note_write(app: AppHandle, state: State<'_, AppState>, path: String, content: String) -> Result<(), String> {
    let event = with_vault_mut(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        // A block type or a skill open in the editor: configuration, written by path
        // and never indexed. The watcher raises whatever event the directory deserves
        // (TypesChanged, or nothing), the same as for an external edit.
        if vemry_core::is_meta_id(&id) {
            return v.write_meta(&id, &content).map(|()| None).map_err(crate::i18n::err);
        }
        v.write_note_as(&id, &content, vemry_core::Writer::Editor).map(Some).map_err(crate::i18n::err)
    })?;
    if let Some(event) = event {
        broadcast(&app, &event);
    }
    Ok(())
}

#[tauri::command]
pub fn note_create(
    app: AppHandle,
    state: State<'_, AppState>,
    title: String,
    folder: Option<String>,
) -> Result<String, String> {
    let id = with_vault_mut(&state, |v| v.create_note(&title, folder.as_deref()).map_err(crate::i18n::err))?;
    remember_open_vault(&app, &state);
    broadcast(&app, &VaultEvent::NoteAdded { id: id.clone() });
    Ok(id.to_string())
}

/// Located insertion (shared by clipping and quick memos): a matching section lands
/// at the end of that section, a missing or unspecified one at the end of the file
/// (with blank lines normalised). Returns where it actually landed, which is how
/// the frontend knows to say "that section does not exist".
/// One kept state of a note, as the history dialog lists it.
#[derive(Serialize)]
pub struct HistorySnapshot {
    pub id: String,
    pub ts_ms: i64,
    pub writer: vemry_core::Writer,
    pub bytes: u64,
}

#[tauri::command]
pub fn note_history_list(state: State<'_, AppState>, path: String) -> Result<Vec<HistorySnapshot>, String> {
    with_vault(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        Ok(v.note_history(&id)
            .into_iter()
            .map(|s| HistorySnapshot { id: s.id, ts_ms: s.ts_ms, writer: s.writer, bytes: s.bytes })
            .collect())
    })
}

/// A kept state next to its difference from the note as it is on disk now — the
/// diff is what the dialog shows, the content is what a copy takes.
#[derive(Serialize)]
pub struct HistoryEntry {
    pub content: String,
    pub diff: String,
}

#[tauri::command]
pub fn note_history_read(state: State<'_, AppState>, path: String, snapshot: String) -> Result<HistoryEntry, String> {
    with_vault(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        let content = v.note_history_read(&id, &snapshot).map_err(crate::i18n::err)?;
        let current = v.read_note(&id).unwrap_or_default();
        // The diff reads "what a restore would change": from now back to the kept state
        let diff = vemry_agent::tools::unified_diff(&current, &content, id.as_str());
        Ok(HistoryEntry { content, diff })
    })
}

#[tauri::command]
pub fn note_history_restore(app: AppHandle, state: State<'_, AppState>, path: String, snapshot: String) -> Result<(), String> {
    let event = with_vault_mut(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        v.note_history_restore(&id, &snapshot).map_err(crate::i18n::err)
    })?;
    broadcast(&app, &event);
    Ok(())
}

/// The quit handshake's question (see `AppState::quit_asked`): the first time, the
/// main webview is asked to write its editor buffer out and `true` says "refuse
/// this request"; every later time the request passes. A refused request that
/// gets no answer (the webview hung) is unblocked by the next one.
pub(crate) fn ask_before_quit(app: &AppHandle) -> bool {
    use std::sync::atomic::Ordering;
    let state: State<'_, AppState> = app.state();
    if state.quit_asked.swap(true, Ordering::SeqCst) {
        return false;
    }
    if app.emit_to("main", "vemry://quit-requested", ()).is_err() {
        // Nobody to ask: let the request through, and the next one too
        return false;
    }
    true
}

/// The main webview's answer to `vemry://quit-requested`: its buffer is on disk,
/// so the app exits — with a code, which the exit handler lets pass.
#[tauri::command]
pub fn quit_confirmed(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
pub fn note_insert(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    text: String,
    section: Option<String>,
) -> Result<InsertLanded, String> {
    let (event, landed) = with_vault_mut(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        v.insert_note(&id, &text, section.as_deref()).map_err(crate::i18n::err)
    })?;
    broadcast(&app, &event);
    Ok(landed)
}

#[tauri::command]
pub fn search(state: State<'_, AppState>, query: String, limit: Option<usize>) -> Result<Vec<SearchHit>, String> {
    with_vault(&state, |v| Ok(v.search(&query, limit.unwrap_or(30))))
}

#[derive(Serialize)]
pub struct OutgoingLink {
    pub raw_target: String,
    pub resolved: Option<String>,
}

#[derive(Serialize)]
pub struct NoteLinks {
    pub outgoing: Vec<OutgoingLink>,
    pub backlinks: Vec<BacklinkEntry>,
}

#[tauri::command]
pub fn links_get(state: State<'_, AppState>, path: String) -> Result<NoteLinks, String> {
    with_vault(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        let outgoing = v
            .index()
            .links_out(&id)
            .into_iter()
            .map(|(e, _)| OutgoingLink {
                raw_target: e.raw_target,
                resolved: e.resolved.map(|t| t.to_string()),
            })
            .collect();
        Ok(NoteLinks { outgoing, backlinks: v.backlinks(&id) })
    })
}

#[tauri::command]
pub fn query_blocks(state: State<'_, AppState>, query: BlockQuery) -> Result<Vec<BlockHit>, String> {
    with_vault(&state, |v| Ok(v.index().query(&query)))
}

/// Whole-vault link graph for the graph view. Human-only: not an agent tool,
/// so private notes appear like everywhere else in the UI.
#[tauri::command]
pub fn graph_get(state: State<'_, AppState>) -> Result<GraphData, String> {
    with_vault(&state, |v| Ok(v.index().graph()))
}

#[tauri::command]
pub fn link_targets(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(v.index().link_targets()))
}

/// The vault's block types (`.vemry/types/*.md`, disabled ones included; the
/// frontend's menus filter). Read live from disk: the editor refreshes on
/// `types_changed`.
#[tauri::command]
pub fn types_list(state: State<'_, AppState>) -> Result<Vec<vemry_core::BlockType>, String> {
    with_vault(&state, |v| Ok(vemry_core::list_block_types(&v.meta_dir())))
}

/// What a scaffold command (`type_create`, `skill_create`, `skill_customize`) hands back: the
/// vault-relative path of the file, and whether it was written just now. A file
/// the user already has is left untouched and reported with `created: false`
/// rather than as an error — it is the end state the command exists to reach, so
/// the frontend opens it instead of complaining.
#[derive(Serialize)]
pub struct Scaffolded {
    pub path: String,
    pub created: bool,
}

/// A scaffold's write: the meta file at vault-relative `rel` (`.vemry/types/*.md`,
/// `.vemry/skills/*.md`) is written unless one is already there. It goes through
/// `Vault::write_meta_if_absent` — nothing but the vault writes under `.vemry/`, and
/// the directory is created by that write, never by merely opening a vault (see
/// `Vault::meta_dir`). Returns whether it wrote.
fn scaffold_meta(state: &State<'_, AppState>, rel: &str, content: &str) -> Result<bool, String> {
    with_vault_mut(state, |v| {
        let id = v.note_id(rel).map_err(crate::i18n::err)?;
        v.write_meta_if_absent(&id, content).map_err(crate::i18n::err)
    })
}

/// Open-or-create for "block type…": the type file for `name`
/// (`.vemry/types/<stem>.md`, the stem sanitised the way a note title is) is
/// written when there is none — `book` gets the full example, any other name the
/// skeleton — and reported as `created`; one already there is left as it is. The
/// frontend opens it either way. A fresh write invalidates the prompt snapshot
/// and is broadcast, the same as an external edit under the directory.
#[tauri::command]
pub fn type_create(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<Scaffolded, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.typeNameEmpty"));
    }
    let stem = vemry_core::sanitize_filename(name);
    let content =
        if stem.eq_ignore_ascii_case("book") { vemry_core::BOOK_TYPE_TEMPLATE.to_string() } else { vemry_core::type_skeleton(name) };
    let rel = format!("{}/{stem}.md", vemry_core::vault::TYPES_DIR);
    let created = scaffold_meta(&state, &rel, &content)?;
    if created {
        broadcast(&app, &VaultEvent::TypesChanged);
    }
    Ok(Scaffolded { path: rel, created })
}

/// The "block type…" row's disable/enable: the file's `disabled:` line is rewritten
/// (`set_frontmatter_disabled`) and written through `Vault::write_meta` like an editor save.
/// `file` must be one of `types_list`'s — the listing is what refuses a symlink — and a
/// frontmatter too damaged to carry the key is reported rather than left as it was.
/// `TypesChanged` is broadcast here too, so the row flips ahead of the watcher's echo.
#[tauri::command]
pub fn type_set_disabled(
    app: AppHandle,
    state: State<'_, AppState>,
    file: String,
    disabled: bool,
) -> Result<(), String> {
    let lang = crate::i18n::current();
    with_vault_mut(&state, |v| {
        if !vemry_core::list_block_types(&v.meta_dir()).iter().any(|t| t.file == file) {
            return Err(crate::i18n::t(lang, "ipc.typeNotFound"));
        }
        let id = v.note_id(&format!("{}/{file}", vemry_core::vault::TYPES_DIR)).map_err(crate::i18n::err)?;
        let next = vemry_core::set_frontmatter_disabled(&v.read_note(&id).map_err(crate::i18n::err)?, disabled);
        if vemry_core::parse_block_type(&file, &next).disabled != disabled {
            return Err(crate::i18n::t(lang, "ipc.typeFrontmatterBroken"));
        }
        v.write_meta(&id, &next).map_err(crate::i18n::err)
    })?;
    broadcast(&app, &VaultEvent::TypesChanged);
    Ok(())
}

/// Inline-field keys and their frequent values for the editor's `key::` completion;
/// notes in a privacy zone contribute nothing (the same courtesy as the tag list).
#[tauri::command]
pub fn fields_list(state: State<'_, AppState>) -> Result<Vec<vemry_core::FieldStat>, String> {
    with_vault(&state, |v| Ok(v.index().field_stats(|id| v.is_private(id), 20)))
}

#[tauri::command]
pub fn folders_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(v.list_folders()))
}

#[tauri::command]
pub fn folder_create(state: State<'_, AppState>, path: String) -> Result<String, String> {
    with_vault(&state, |v| v.create_folder(&path).map_err(crate::i18n::err))
}

#[tauri::command]
pub fn files_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(v.list_files()))
}

#[tauri::command]
pub fn file_create(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<String, String> {
    let rel = with_vault_mut(&state, |v| v.create_file(&path).map_err(crate::i18n::err))?;
    if rel.ends_with(".md") {
        broadcast(&app, &VaultEvent::NoteAdded { id: vemry_core::NoteId::new(&rel) });
    }
    Ok(rel)
}

#[tauri::command]
pub fn path_rename(app: AppHandle, state: State<'_, AppState>, from: String, to: String) -> Result<String, String> {
    let (rel, note_count) = with_vault_mut(&state, |v| {
        let rel = v.rename_path(&from, &to).map_err(crate::i18n::err)?;
        Ok((rel, v.index().note_count()))
    })?;
    broadcast(&app, &VaultEvent::IndexReady { note_count });
    Ok(rel)
}

#[tauri::command]
pub fn path_delete(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<(), String> {
    let note_count = with_vault_mut(&state, |v| {
        v.delete_path(&path).map_err(crate::i18n::err)?;
        Ok(v.index().note_count())
    })?;
    broadcast(&app, &VaultEvent::IndexReady { note_count });
    Ok(())
}

/// Read a non-Markdown vault file as UTF-8 text (errors on binary/oversized —
/// the frontend then falls back to `file_open_system`).
#[tauri::command]
pub fn file_read(state: State<'_, AppState>, path: String) -> Result<String, String> {
    with_vault(&state, |v| v.read_file(&path).map_err(crate::i18n::err))
}

/// Raw bytes of a vault file (image/PDF preview), sent over the binary IPC
/// channel. Oversized files error — the frontend falls back to the OS app.
#[tauri::command]
pub fn file_read_binary(state: State<'_, AppState>, path: String) -> Result<tauri::ipc::Response, String> {
    let bytes = with_vault(&state, |v| v.read_file_bytes(&path).map_err(crate::i18n::err))?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Save pasted image/file bytes as a vault attachment. The bytes arrive as the
/// raw IPC body; the suggested relative path rides in the `path` header,
/// percent-encoded by api.ts (encodeURIComponent) because header values must be
/// ISO-8859-1 — a raw Chinese filename crashes `new Headers()` on Windows
/// WebView2. Returns the final (deduped) relative path.
#[tauri::command]
pub fn attachment_save(state: State<'_, AppState>, request: tauri::ipc::Request<'_>) -> Result<String, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw bytes".into());
    };
    let path = request
        .headers()
        .get("path")
        .and_then(|v| v.to_str().ok())
        .ok_or("missing path header")?;
    let path = decode_path_header(path)?;
    with_vault(&state, |v| v.save_attachment(&path, bytes).map_err(crate::i18n::err))
}

/// Decode the percent-encoded path in an IPC header (api.ts encodes it with
/// encodeURIComponent).
/// api.ts is the only producer, and the two sides must change together — this
/// decodes unconditionally, so a literal `%` in a file name round-trips only
/// because the frontend encodes it as `%25` first.
fn decode_path_header(raw: &str) -> Result<String, String> {
    percent_encoding::percent_decode_str(raw)
        .decode_utf8()
        .map(|s| s.into_owned())
        .map_err(|_| "path header is not valid UTF-8".to_string())
}

/// Paste the system clipboard into a vault folder (`dest`, "" = root):
/// Finder-copied files/folders import by fs copy (any size or type, original
/// names, deduped); otherwise raw clipboard image bytes (screenshots) save as
/// `<stamp>.png`. `reuse_vault_files` = paste-into-note semantics: sources
/// already inside the vault are referenced (path returned) instead of copied.
/// Returns the created vault-relative paths — empty when the clipboard holds
/// neither files nor an image, so the caller can fall back to DOM paste data.
#[tauri::command]
pub fn clipboard_paste(
    state: State<'_, AppState>,
    dest: String,
    stamp: String,
    reuse_vault_files: bool,
) -> Result<Vec<String>, String> {
    let files = crate::pasteboard::file_paths();
    if !files.is_empty() {
        return with_vault_mut(&state, |v| {
            v.import_files(&files, &dest, reuse_vault_files).map_err(crate::i18n::err)
        });
    }
    if let Some((bytes, ext)) = crate::pasteboard::image_data() {
        let name = format!("{stamp}.{ext}");
        let rel = if dest.is_empty() { name } else { format!("{dest}/{name}") };
        return with_vault(&state, |v| {
            v.save_attachment(&rel, &bytes).map(|p| vec![p]).map_err(crate::i18n::err)
        });
    }
    Ok(vec![])
}

/// Import files the user picked from the native file dialog into a vault folder
/// (the agent composer's "+" attachment): the same primitive as the Finder branch
/// of `clipboard_paste`, with `reuse_in_vault`=true — a chosen file that is already
/// inside the vault has its path returned without being copied (the same semantics
/// as pasting into a note). An explicit user action rather than an AI write path,
/// so it does not go through the PermissionGate. Returns vault-relative paths in
/// the order they were given.
#[tauri::command]
pub fn files_import(state: State<'_, AppState>, paths: Vec<String>, dest: String) -> Result<Vec<String>, String> {
    let sources: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();
    with_vault_mut(&state, |v| v.import_files(&sources, &dest, true).map_err(crate::i18n::err))
}

/// Plain text on the clipboard: the second of the three fallbacks behind the
/// editor's right-click "paste" (after files and screenshots).
/// Off macOS it returns None and the frontend falls back to
/// navigator.clipboard.readText().
/// The Markdown formatter as a command: the editor's "format note" runs the note's
/// text through it (the whole note, at the user's explicit request — never on
/// autosave) and replaces the document with the result.
#[tauri::command]
pub fn format_markdown(app: AppHandle, text: String) -> String {
    let settings: Settings = settings::load(&app);
    vemry_core::format_markdown(&text, &settings.format_options())
}

/// A rich-text paste into the editor: the clipboard's HTML fragment → Markdown (the
/// clip layer's converter, the same one a clipped selection goes through) →
/// formatted when the clip formatter is on. Empty when nothing converts (the caller
/// falls back to the plain text).
#[tauri::command]
pub fn html_to_markdown(app: AppHandle, html: String) -> String {
    let md = vemry_clip::fragment_to_markdown(&html);
    if md.trim().is_empty() {
        return String::new();
    }
    let settings: Settings = settings::load(&app);
    settings::format_or(&md, settings.clip_format()).into_owned()
}

#[tauri::command]
pub fn clipboard_read_text() -> Option<String> {
    crate::pasteboard::string_text()
}

/// The files on the system clipboard (a file manager's copy) as absolute paths —
/// the capture window's paste. The window keeps the paths until Enter decides
/// which note's attachment bucket they go to (`clipboard_paste` writes at paste
/// time, which needs a destination that is not known yet); they are then imported
/// through `files_import`. Empty off macOS, where the DOM paste data is the fallback.
#[tauri::command]
pub fn clipboard_files() -> Vec<String> {
    crate::pasteboard::file_paths().into_iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

/// Raw image bytes on the system clipboard (a screenshot), over the binary
/// channel; empty when there is none. The same deferred-destination reason as
/// `clipboard_files`: the bytes are held by the capture window and saved with
/// `attachment_save` once the target is known. The format is sniffed by the caller
/// from the bytes (PNG is preferred by the reader, TIFF is the other possibility).
#[tauri::command]
pub fn clipboard_image() -> tauri::ipc::Response {
    tauri::ipc::Response::new(crate::pasteboard::image_data().map(|(b, _)| b).unwrap_or_default())
}

/// One-click copy of an editor image: an image file in the vault is written to the
/// system clipboard as a bitmap (the reverse of clipboard_paste). Decoding is left
/// to NSImage, and a failure to decode or a non-macOS platform is an error — the
/// button reports it rather than failing silently.
#[tauri::command]
pub fn clipboard_copy_image(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let bytes = with_vault(&state, |v| v.read_file_bytes(&path).map_err(crate::i18n::err))?;
    if crate::pasteboard::write_image(&bytes) {
        Ok(())
    } else if cfg!(target_os = "macos") {
        Err(crate::i18n::t(crate::i18n::current(), "ipc.imageFormatUnsupported"))
    } else {
        // Off macOS write_image is a stub that is always false: the capability is absent, the image is not broken
        Err(crate::i18n::t(crate::i18n::current(), "ipc.copyImageUnsupported"))
    }
}

/// Overwrite a non-Markdown vault file. `.md` is rejected — notes go through
/// `note_write` so the index stays consistent. `write_file` now suppresses its
/// own watcher echo, so broadcast the event explicitly like the note paths do
/// (frontend refresh + workflow dispatch for user-initiated writes).
#[tauri::command]
pub fn file_write(app: AppHandle, state: State<'_, AppState>, path: String, content: String) -> Result<(), String> {
    with_vault_mut(&state, |v| v.write_file(&path, &content).map_err(crate::i18n::err))?;
    broadcast(&app, &VaultEvent::FilesChanged);
    Ok(())
}

/// Open a non-Markdown vault file with the OS default application.
#[tauri::command]
pub fn file_open_system(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let abs = with_vault(&state, |v| {
        let id = v.note_id(&path); // reuses escape validation; ignore .md suffixing
        let _ = id;
        let rel = path.trim().trim_matches('/');
        if rel.is_empty() || rel.split('/').any(|s| s == "..") || path.starts_with('/') {
            return Err("invalid path".into());
        }
        Ok(v.root().join(rel))
    })?;
    opener::open(abs).map_err(|e| e.to_string())
}

/// Open an external http(s) URL with the OS default browser (markdown links
/// in notes; vault files go through `file_open_system` instead).
#[tauri::command]
pub fn url_open_system(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(crate::i18n::tf(crate::i18n::current(), "ipc.httpOnly", &[("url", &url)]));
    }
    opener::open(url).map_err(|e| e.to_string())
}

/// Resolve the inside of a `[[…]]` — note, `Note#Heading`, `Note#^id` or
/// `#^id` — to the note and the slice it denotes (see `Vault::resolve_ref`).
/// Human-only like `resolve_link`: private notes are not filtered.
#[tauri::command]
pub fn resolve_ref(state: State<'_, AppState>, target: String, from: Option<String>) -> Result<RefResolution, String> {
    with_vault(&state, |v| {
        let from_id = match from {
            Some(f) => Some(v.note_id(&f).map_err(crate::i18n::err)?),
            None => None,
        };
        v.resolve_ref(&target, from_id.as_ref()).map_err(crate::i18n::err)
    })
}

/// Give a block on disk a stable `^id` (minted on demand) and return it.
/// `local_id` and `guard` come from a query hit, so an edited note refuses
/// instead of tagging the wrong block.
#[tauri::command]
pub fn block_ensure_id(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
    local_id: String,
    guard: Option<String>,
) -> Result<String, String> {
    let (id, event) = with_vault_mut(&state, |v| {
        let nid = v.note_id(&path).map_err(crate::i18n::err)?;
        v.ensure_block_id(&nid, &local_id, guard.as_deref()).map_err(crate::i18n::err)
    })?;
    if let Some(ev) = event {
        broadcast(&app, &ev);
    }
    Ok(id)
}

/// The editor-buffer half of id minting: the frontend sends the text it
/// holds (a disk write would be clobbered by autosave) and gets back the
/// one insert to apply. Pure — no vault lock, nothing written.
#[tauri::command]
pub fn block_id_plan(source: String, selector: BlockSelector) -> Result<BlockIdPlan, String> {
    vemry_core::plan_block_id(&source, &selector).map_err(crate::i18n::err)
}

/// A note the agent wrote that quotes a page — `source:` in its frontmatter and
/// images still pointing at that page (a paper's figures archived by the study
/// skill) — gets the clip's image localisation: the figures are downloaded into
/// clips/assets/ and the note rewritten to reference them, the way a clip's are
/// at the moment of clipping. The user approved the write and, for the read, the
/// page; fetching the page's figures is within that. It honours the localisation
/// setting, and a note with nothing remote is left alone (the rewrite comes back
/// through here once more and finds nothing to do).
fn localize_agent_figures(app: &AppHandle, rel: &str) {
    if !rel.ends_with(".md") || !crate::settings::load(app).clip_localize_images {
        return;
    }
    let state = app.state::<AppState>();
    // The index already holds the frontmatter `source:` — this runs on every agent write,
    // on the event drain, so it must not read and parse the note to learn it. Whether the
    // note actually references anything remote is localize_clip_images' own first question
    let source = with_vault(&state, |v| {
        let id = v.note_id(rel).map_err(crate::i18n::err)?;
        Ok(v.index().summary_ref(&id).and_then(|s| s.source.clone()).filter(|u| u.starts_with("http")))
    });
    if let Ok(Some(url)) = source {
        crate::browser::clip::maybe_localize(app, rel.to_string(), url);
    }
}

/// Who authored each block of a note (see vemry-core's provenance.rs).
#[tauri::command]
pub fn provenance_get(state: State<'_, AppState>, path: String) -> Result<Vec<BlockProvenance>, String> {
    with_vault(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        v.provenance(&id).map_err(crate::i18n::err)
    })
}

/// Accept the pending agent blocks of a note (those with these local ids — a
/// run — or all of them); answers with how many blocks it accepted. Not
/// broadcast: accepting is not a note event.
#[tauri::command]
pub fn provenance_accept(state: State<'_, AppState>, path: String, local_ids: Option<Vec<String>>) -> Result<usize, String> {
    with_vault_mut(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        v.accept_provenance_ids(&id, local_ids.as_deref()).map_err(crate::i18n::err)
    })
}

/// One note holding agent blocks awaiting review, with their count.
#[derive(serde::Serialize)]
pub struct PendingNote {
    pub path: String,
    pub blocks: usize,
}

/// The notes with unreviewed agent blocks across the whole vault, in path order:
/// the status bar's list, the file tree's marks and the review's note-to-note
/// step all read it.
#[tauri::command]
pub fn provenance_pending_notes(state: State<'_, AppState>) -> Result<Vec<PendingNote>, String> {
    with_vault(&state, |v| Ok(v.pending_notes().into_iter().map(|(path, blocks)| PendingNote { path, blocks }).collect()))
}

/// What a note-level accept did: agent blocks accepted, and the notes touched.
#[derive(serde::Serialize)]
pub struct AcceptedTotal {
    pub blocks: usize,
    pub notes: usize,
}

/// Accept every unreviewed agent block in these notes (every note, when None).
/// Not broadcast: accepting is not a note event.
#[tauri::command]
pub fn provenance_accept_notes(state: State<'_, AppState>, paths: Option<Vec<String>>) -> Result<AcceptedTotal, String> {
    with_vault_mut(&state, |v| {
        let (blocks, notes) = v.accept_provenance_notes(paths.as_deref());
        Ok(AcceptedTotal { blocks, notes })
    })
}

/// How to undo the agent writes behind the unreviewed blocks of the note in
/// `text` (the editor's buffer) — the blocks with these local ids (a run), or
/// all of them — as byte replacements in that text. Nothing is written: the
/// editor applies the plan as one undoable edit and saves as it does any edit.
#[tauri::command]
pub fn provenance_revert_plan(state: State<'_, AppState>, text: String, local_ids: Option<Vec<String>>) -> Result<RevertPlan, String> {
    with_vault(&state, |v| {
        v.revert_plan_ids(&text, local_ids.as_deref()).map_err(crate::i18n::err)
    })
}

/// Record text the user placed themselves (a chat answer inserted, a clip, a
/// capture) — it lands accepted. A private path records nothing (the ledger
/// would name it); text without blocks records nothing.
#[tauri::command]
pub fn provenance_record(
    state: State<'_, AppState>,
    path: String,
    text: String,
    origin: String,
    session: Option<String>,
    url: Option<String>,
) -> Result<(), String> {
    let origin = match origin.as_str() {
        "agent" => InsertOrigin::Agent,
        "clip" => InsertOrigin::Clip,
        "capture" => InsertOrigin::Capture,
        other => return Err(format!("unknown origin: {other}")),
    };
    with_vault_mut(&state, |v| {
        let id = v.note_id(&path).map_err(crate::i18n::err)?;
        if v.is_private(&id) {
            return Ok(());
        }
        let blocks = vemry_core::stamps_of(&text);
        if blocks.is_empty() {
            return Ok(());
        }
        v.record_provenance(ProvenanceEntry::Insert {
            ts: vemry_core::provenance::now_ts(),
            origin,
            session,
            url,
            path: id.to_string(),
            blocks,
        });
        Ok(())
    })
}

/// Resolve a wikilink target to a note path, if it exists.
#[tauri::command]
pub fn resolve_link(state: State<'_, AppState>, target: String, from: Option<String>) -> Result<Option<String>, String> {
    with_vault(&state, |v| {
        let from_id = match from {
            Some(f) => v.note_id(&f).map_err(crate::i18n::err)?,
            None => vemry_core::NoteId::new("_.md"),
        };
        Ok(v.index().resolve_target(&target, &from_id).map(|id| id.to_string()))
    })
}

#[derive(Serialize)]
pub struct SettingsView {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub env_key_present: bool,
    pub recent_vaults: Vec<String>,
    pub language: settings::LangSetting,
    pub adblock_enabled: bool,
    pub adblock_rule_urls: Vec<String>,
    pub open_links_in: settings::LinkOpenMode,
    pub click_opens_links: bool,
    /// A read-only mirror: the shape a browser window is built in (detached or
    /// docked). The write path is browser_set_docked (every toggle is persisted =
    /// remember the last choice), which does not go through SettingsUpdate and has
    /// no switch in the settings page.
    pub browser_docked: bool,
    pub web_search_enabled: bool,
    /// Derived and read-only: the provider runs the search (the DeepSeek endpoint;
    /// decided next to the other provider predicates, not reimplemented in TS). The
    /// composer's web search switch is enabled by it.
    pub web_search_available: bool,
    /// Derived and read-only: the switch is on and the provider runs the search (the
    /// same source as agent_send's per-turn check).
    pub web_search_active: bool,
    /// The agent's plan confirmation (propose_plan plus a blocking gate); frozen per
    /// session, so a change takes effect from the next session.
    pub plan_confirm: bool,
    pub provenance_marks: bool,
    pub reading_images_enabled: bool,
    pub clip_localize_images: bool,
    pub format_agent_writes: bool,
    pub format_clips: bool,
    pub format_cjk_spacing: bool,
    pub format_punctuation: bool,
    pub updater_auto_check: bool,
    /// The version "skip this version" muted for the silent startup check (None = none).
    pub updater_skipped_version: Option<String>,
    pub note_font_size: u8,
    pub readable_line_width: bool,
    pub reasoning_effort: String,
    /// Derived and read-only: effort is sent only when the endpoint is DeepSeek (the
    /// same "the check is not reimplemented in TS" as web_search_active).
    pub reasoning_effort_active: bool,
    /// Derived and read-only: the selected model has no image understanding
    /// (DeepSeek's pro model) — the frontend warns once when the composer attaches an
    /// image; the check is not reimplemented in TS.
    pub images_unsupported: bool,
    /// Side-by-side translation: the target language ("" = the interface language),
    /// the model ("" = the chat model) and the user's extra rule.
    pub translate_target: String,
    pub translate_model: String,
    pub translate_addendum: String,
    /// Which provider the AI talks to (custom = bring your own key, hosted = vemry
    /// hosted) and the hosted model choice ("" = the catalog's default).
    pub provider: settings::ProviderSetting,
    pub hosted_model: String,
    /// Derived and read-only: the model requests actually name (the research
    /// panel's header) — the hosted choice or the catalog's default in hosted mode,
    /// `model` otherwise.
    pub active_model: String,
    /// Quick capture: the global shortcut (the plugin's grammar), the registration's
    /// failure if any (derived and read-only — None means the shortcut works), and
    /// the landing target with its pin.
    pub capture_hotkey: String,
    pub capture_hotkey_error: Option<String>,
    pub capture_target: settings::CaptureTarget,
    pub capture_pinned_path: String,
    pub capture_pinned_heading: String,
    /// The resident state (closing the main window hides it behind a tray icon),
    /// and the icon's creation failure if any (derived and read-only — None means
    /// the icon is there, or the switch is off).
    pub tray_resident: bool,
    pub tray_error: Option<String>,
}

/// A partial update: `None` = keep the existing value. The frontend fetches
/// everything with settingsGet and sends it back after editing.
#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub base_url: Option<String>,
    pub provider: Option<settings::ProviderSetting>,
    pub hosted_model: Option<String>,
    pub language: Option<settings::LangSetting>,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub adblock_enabled: Option<bool>,
    pub adblock_rule_urls: Option<Vec<String>>,
    pub open_links_in: Option<settings::LinkOpenMode>,
    pub click_opens_links: Option<bool>,
    pub web_search_enabled: Option<bool>,
    pub plan_confirm: Option<bool>,
    pub provenance_marks: Option<bool>,
    pub reading_images_enabled: Option<bool>,
    pub clip_localize_images: Option<bool>,
    pub format_agent_writes: Option<bool>,
    pub format_clips: Option<bool>,
    pub format_cjk_spacing: Option<bool>,
    pub format_punctuation: Option<bool>,
    pub updater_auto_check: Option<bool>,
    /// Set-only: the dialog's "skip this version". Nothing clears it — a later
    /// release differs from it, and a check the user asks for ignores it anyway.
    pub updater_skipped_version: Option<String>,
    pub note_font_size: Option<u8>,
    pub readable_line_width: Option<bool>,
    pub reasoning_effort: Option<String>,
    pub translate_target: Option<String>,
    pub translate_model: Option<String>,
    pub translate_addendum: Option<String>,
    pub capture_hotkey: Option<String>,
    pub capture_target: Option<settings::CaptureTarget>,
    pub capture_pinned_path: Option<String>,
    pub capture_pinned_heading: Option<String>,
    pub tray_resident: Option<bool>,
}

/// VEMRY.md's starter content in the current language: the shared header plus
/// the example body. Both halves live in `locales/*.json`, so core stays free of
/// copy and the two languages cannot drift apart.
fn memory_template(app: &AppHandle) -> String {
    let lang = crate::i18n::resolve(settings::load(app).language);
    format!("{}\n{}", crate::i18n::t(lang, "memoryFile.header"), crate::i18n::t(lang, "memoryFile.template"))
}

#[tauri::command]
pub fn settings_get(app: AppHandle) -> SettingsView {
    let s = settings::load(&app);
    let web_search_available = s.provider_config().web_search_available();
    let web_search_active = s.web_search_active();
    let reasoning_effort_active = s.reasoning_active();
    let images_unsupported = s.provider_config().images_unsupported();
    let active_model = s.active_model();
    SettingsView {
        base_url: s.base_url.clone(),
        model: s.model.clone(),
        api_key: s.api_key.clone(),
        env_key_present: std::env::var("ZHIPU_API_KEY").map(|k| !k.is_empty()).unwrap_or(false),
        recent_vaults: s.recent_vaults,
        language: s.language,
        adblock_enabled: s.adblock_enabled,
        adblock_rule_urls: s.adblock_rule_urls,
        open_links_in: s.open_links_in,
        click_opens_links: s.click_opens_links,
        browser_docked: s.browser_docked,
        web_search_enabled: s.web_search_enabled,
        web_search_available,
        web_search_active,
        plan_confirm: s.plan_confirm,
        provenance_marks: s.provenance_marks,
        reading_images_enabled: s.reading_images_enabled,
        clip_localize_images: s.clip_localize_images,
        format_agent_writes: s.format_agent_writes,
        format_clips: s.format_clips,
        format_cjk_spacing: s.format_cjk_spacing,
        format_punctuation: s.format_punctuation,
        updater_auto_check: s.updater_auto_check,
        updater_skipped_version: s.updater_skipped_version,
        note_font_size: s.note_font_size,
        readable_line_width: s.readable_line_width,
        reasoning_effort: s.reasoning_effort,
        reasoning_effort_active,
        images_unsupported,
        translate_target: s.translate_target,
        translate_model: s.translate_model,
        translate_addendum: s.translate_addendum,
        provider: s.provider,
        hosted_model: s.hosted_model,
        active_model,
        capture_hotkey: s.capture_hotkey,
        capture_hotkey_error: crate::capture_window::hotkey_error(&app),
        capture_target: s.capture_target,
        capture_pinned_path: s.capture_pinned_path,
        capture_pinned_heading: s.capture_pinned_heading,
        tray_resident: s.tray_resident,
        tray_error: crate::tray::error(&app),
    }
}

#[tauri::command]
pub fn settings_set(app: AppHandle, update: SettingsUpdate) -> Result<(), String> {
    let mut s = settings::load(&app);
    if let Some(base_url) = update.base_url {
        s.base_url =
            if base_url.trim().is_empty() { vemry_agent::provider::ZHIPU_BASE_URL.into() } else { base_url };
    }
    if let Some(model) = update.model {
        s.model = if model.trim().is_empty() { vemry_agent::provider::DEFAULT_MODEL.into() } else { model };
    }
    if let Some(api_key) = update.api_key {
        s.api_key = api_key;
    }
    if let Some(v) = update.provider {
        s.provider = v;
    }
    if let Some(v) = update.hosted_model {
        s.hosted_model = v.trim().to_string();
    }
    let language_changed = update.language.is_some_and(|v| v != s.language);
    if let Some(v) = update.language {
        s.language = v;
    }
    let adblock_toggled = update.adblock_enabled.is_some_and(|v| v != s.adblock_enabled);
    if let Some(v) = update.adblock_enabled {
        s.adblock_enabled = v;
    }
    let mut adblock_urls_changed = false;
    if let Some(urls) = update.adblock_rule_urls {
        let urls: Vec<String> = urls.into_iter().filter(|u| !u.trim().is_empty()).collect();
        adblock_urls_changed = urls != s.adblock_rule_urls;
        s.adblock_rule_urls = urls;
    }
    if let Some(mode) = update.open_links_in {
        s.open_links_in = mode;
    }
    if let Some(v) = update.click_opens_links {
        s.click_opens_links = v;
    }
    if let Some(v) = update.web_search_enabled {
        s.web_search_enabled = v;
    }
    if let Some(v) = update.plan_confirm {
        s.plan_confirm = v;
    }
    if let Some(v) = update.provenance_marks {
        s.provenance_marks = v;
    }
    if let Some(v) = update.reading_images_enabled {
        s.reading_images_enabled = v;
    }
    if let Some(v) = update.format_agent_writes {
        s.format_agent_writes = v;
    }
    if let Some(v) = update.format_clips {
        s.format_clips = v;
    }
    if let Some(v) = update.format_cjk_spacing {
        s.format_cjk_spacing = v;
    }
    if let Some(v) = update.format_punctuation {
        s.format_punctuation = v;
    }
    if let Some(v) = update.clip_localize_images {
        s.clip_localize_images = v;
    }
    if let Some(v) = update.updater_auto_check {
        s.updater_auto_check = v;
    }
    if let Some(v) = update.updater_skipped_version {
        s.updater_skipped_version = Some(v);
    }
    if let Some(v) = update.note_font_size {
        s.note_font_size = v.clamp(15, 18);
    }
    if let Some(v) = update.readable_line_width {
        s.readable_line_width = v;
    }
    if let Some(v) = update.reasoning_effort {
        // The same clamp as note_font_size: an invalid value falls back to the default, so a hand edit or a bad round trip cannot go out of range
        s.reasoning_effort = if settings::is_reasoning_effort(&v) { v } else { "high".into() };
    }
    if let Some(v) = update.translate_target {
        s.translate_target = v.trim().to_string();
    }
    if let Some(v) = update.translate_model {
        s.translate_model = v.trim().to_string();
    }
    if let Some(v) = update.translate_addendum {
        s.translate_addendum = v.trim().to_string();
    }
    if let Some(v) = update.capture_target {
        s.capture_target = v;
    }
    if let Some(v) = update.capture_pinned_path {
        s.capture_pinned_path = v.trim().to_string();
    }
    if let Some(v) = update.capture_pinned_heading {
        s.capture_pinned_heading = v.trim().to_string();
    }
    if let Some(v) = update.tray_resident {
        s.tray_resident = v;
    }
    // The shortcut is the one field whose write can fail (another application holds
    // the keys): it is re-registered first and stored only once that worked, after
    // the rest of the update has landed — a refused combination costs the user
    // nothing else they changed. The validation is the plugin's own parser, so a
    // hand-edited settings.json cannot store a combination that never registers.
    let new_hotkey = update.capture_hotkey.map(|v| v.trim().to_string()).filter(|v| *v != s.capture_hotkey);
    if let Some(v) = &new_hotkey {
        if !crate::capture_window::is_valid_hotkey(v) {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.captureHotkeyInvalid"));
        }
    }
    settings::save(&app, &s)?;
    if let Some(v) = new_hotkey {
        crate::capture_window::rebind(&app, &s.capture_hotkey, &v)?;
        let _ = settings::update(&app, |s| {
            s.capture_hotkey = v.clone();
            true
        });
        // The menu item's hint carries the combination (macOS; a no-op elsewhere)
        if let Err(e) = crate::menu::install(&app) {
            eprintln!("menu rebuild after shortcut change failed: {e}");
        }
    }
    if adblock_toggled || adblock_urls_changed {
        // An edited source list has to be recompiled: dropping a subscription stops
        // its cached file from being read (rule_files goes by the current list), but
        // the rules already compiled from it stay in force until something rebuilds
        // them — otherwise the list the user removed keeps blocking until a restart.
        crate::browser::adblock::reconcile(&app);
    }
    if adblock_toggled {
        // Switching it on is the user asking for blocking: with nothing cached yet,
        // fetch the subscriptions now rather than leaving them on a "no rules" state
        // that needs a second click. Editing the list is deliberately not a fetch —
        // "update now" sits next to the box, and every network exit of this feature
        // stays tied to something the user asked for.
        crate::browser::adblock::ensure_rules_soon(&app);
    }
    if language_changed {
        crate::i18n::set_current(crate::i18n::resolve(s.language));
        // The native menu is built once from the catalog; rebuilding is the only
        // way to relabel it (set_menu replaces the previous menu wholesale).
        if let Err(e) = crate::menu::install(&app) {
            eprintln!("menu rebuild after language change failed: {e}");
        }
    }
    // The tray icon follows the switch, and its menu the language and the shortcut
    // hint — after every write, so the three cannot drift apart
    crate::tray::reconcile(&app);
    Ok(())
}

/// Read the whole object of .vemry/config.json (missing or broken JSON = None).
/// Reading and writing share this one parse point — `skills::read_vault_config`,
/// which the prompt side reads `skills_disabled` through: write_policy and
/// skills_disabled live in the same file.
fn vault_config_read(vault: &Vault) -> Option<serde_json::Value> {
    vemry_agent::skills::read_vault_config(&vault.meta_dir())
}

/// Read one key from .vemry/config.json.
fn vault_config_get(vault: &Vault, key: &str) -> Option<serde_json::Value> {
    vault_config_read(vault)?.get(key).cloned()
}

fn vault_write_policy(vault: &Vault) -> WritePolicy {
    match vault_config_get(vault, "write_policy").and_then(|p| p.as_str().map(|p| p == "allow")) {
        Some(true) => WritePolicy::Allow,
        _ => WritePolicy::Ask,
    }
}

/// Merge one key into the vault's .vemry/config.json — overwriting the whole file
/// would wipe out the other keys.
fn vault_config_set(vault: &Vault, key: &str, value: serde_json::Value) -> Result<(), String> {
    // meta_dir is a pure accessor now (it creates nothing), so a write path creates the directory itself
    let dir = vault.meta_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("config.json");
    let mut obj = vault_config_read(vault).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    obj.insert(key.into(), value);
    std::fs::write(&path, serde_json::Value::Object(obj).to_string()).map_err(|e| e.to_string())
}

fn persist_allow_always(state: &State<'_, AppState>) {
    let guard = state.vault.lock().unwrap();
    if let Some(vault) = guard.as_ref() {
        let _ = vault_config_set(vault, "write_policy", json!("allow"));
    }
}

/// The current vault's persistent write policy (the data behind the settings page's
/// "writes pre-authorised" badge).
#[tauri::command]
pub fn vault_write_policy_get(state: State<'_, AppState>) -> Result<String, String> {
    with_vault(&state, |v| {
        Ok(match vault_write_policy(v) {
            WritePolicy::Allow => "allow".into(),
            WritePolicy::Ask => "ask".into(),
        })
    })
}

/// Change the persistent write policy: revoke allow_always (back to approving each
/// write) or grant it explicitly.
#[tauri::command]
pub fn vault_write_policy_set(state: State<'_, AppState>, policy: String) -> Result<(), String> {
    if policy != "allow" && policy != "ask" {
        return Err(format!("unknown policy: {policy}"));
    }
    // A revocation takes effect at once: the cross-turn grant of a running session is
    // withdrawn with it, and the session's ledger gets a `grants_cleared` line so the
    // replay (a switch-and-back, the Resume button) does not rebuild the grants out
    // of the earlier allow lines
    if policy == "ask" {
        state.agent.session_grant.lock().unwrap().clear();
        append_session_line(&state, "grants_cleared", json!({}))?;
    }
    with_vault(&state, |v| vault_config_set(v, "write_policy", json!(policy)))
}

/// A web page body the user handed over with "add to context", entering this turn
/// with the message. The envelope assembly and the untrusted-content declaration
/// live on the vemry-agent side (compose_user_message is the one assembly point);
/// this is only the IPC parameter. The context is handed over explicitly by the
/// user, and the agent still has no way to fetch a web page on its own.
#[derive(Deserialize)]
pub struct PageContextArg {
    pub title: String,
    pub url: String,
    pub markdown: String,
}

/// The two ways into a turn: a new message, or a regenerate of the last turn (no
/// user message is appended, the original envelope and web context are kept, and
/// only the generation is replayed).
enum TurnInput {
    New {
        message: String,
        page: Option<vemry_agent::PageContext>,
        /// @ references: path identifiers only, resolved (and privacy-gated) inside the runner's vault lock.
        refs: Vec<vemry_agent::ContextRef>,
        current_note: Option<String>,
    },
    Retry,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)] // IPC parameters are named one by one (a Tauri command signature is the contract); no parameter object
pub async fn agent_start(
    app: AppHandle,
    state: State<'_, AppState>,
    message: String,
    current_note: Option<String>,
    page_context: Option<PageContextArg>,
    context_refs: Option<Vec<vemry_agent::ContextRef>>,
    effort: Option<String>,
    on_event: Channel<AgentEvent>,
) -> Result<String, String> {
    let page = page_context
        .map(|p| vemry_agent::PageContext { url: p.url, title: p.title, markdown: p.markdown });
    let refs = context_refs.unwrap_or_default();
    start_turn(app, state, TurnInput::New { message, page, refs, current_note }, effort, on_event).await
}

/// Regenerate the last answer: cut the assistant/tool tail after the last user
/// message and replay the generation from the original envelope (web context
/// included). A retry marker line is appended to the audit jsonl, and the existing
/// audit lines are untouched (the audit log is immutable).
#[tauri::command]
pub async fn agent_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    effort: Option<String>,
    on_event: Channel<AgentEvent>,
) -> Result<String, String> {
    start_turn(app, state, TurnInput::Retry, effort, on_event).await
}

async fn start_turn(
    app: AppHandle,
    state: State<'_, AppState>,
    input: TurnInput,
    effort: Option<String>,
    on_event: Channel<AgentEvent>,
) -> Result<String, String> {
    if state.agent.running.swap(true, Ordering::SeqCst) {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.agentRunning"));
    }
    state.agent.cancel.store(false, Ordering::SeqCst);

    let result = run_agent_turn(&app, &state, input, effort, &on_event).await;

    state.agent.running.store(false, Ordering::SeqCst);
    state.agent.pending.deny_all();
    match result {
        Ok(session_id) => Ok(session_id),
        Err(e) => {
            let _ = on_event.send(AgentEvent::Error { message: e.clone() });
            Err(e)
        }
    }
}

async fn run_agent_turn(
    app: &AppHandle,
    state: &State<'_, AppState>,
    input: TurnInput,
    effort: Option<String>,
    on_event: &Channel<AgentEvent>,
) -> Result<String, String> {
    let settings: Settings = settings::load(app);
    let hosted = settings.hosted();
    if hosted {
        // The account has to be in hand before a hosted turn goes out: signed in,
        // with hosted access, and a snapshot within the grace. A snapshot older than
        // ten minutes is refreshed first (a stale one still passes if the refresh
        // fails; an expired one does not), so a day-old entitlement never refuses a
        // user whose purchase went through this morning.
        let st = crate::account::status(app);
        if !st.logged_in {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.hosted.signIn"));
        }
        if st.freshness != crate::account::Freshness::Fresh {
            let _ = crate::account::refresh(app).await;
        }
        let st = crate::account::status(app);
        if !st.logged_in {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.hosted.signIn"));
        }
        if st.freshness == crate::account::Freshness::Expired {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.hosted.expired"));
        }
        if !st.hosted_ok {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.hosted.hosted_not_allowed"));
        }
    }
    let provider = settings.provider_config();
    if provider.api_key.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), if hosted { "ipc.hosted.signIn" } else { "ipc.noApiKey" }));
    }

    let (root, meta_dir, write_policy) = {
        let guard = state.vault.lock().unwrap();
        let vault = guard.as_ref().ok_or("no vault open")?;
        (vault.root().to_path_buf(), vault.meta_dir(), vault_write_policy(vault))
    };

    // Decide the cross-vault case before settling on a session file: on a vault switch the old session id must not follow into the new vault's sessions/
    let mut history = state.agent.history.lock().await;
    {
        let mut owner = state.agent.history_vault.lock().unwrap();
        if owner.as_deref() != Some(root.as_path()) {
            history.clear();
            *state.agent.current_session.lock().unwrap() = None;
            state.agent.reset_session_scope();
            state.agent.clear_session_data();
            *owner = Some(root);
        }
    }
    // Regenerate: truncate to the last user message (inclusive), discarding the assistant/tool tail after it
    if matches!(input, TurnInput::Retry) {
        let Some(idx) = vemry_agent::compact::last_user_index(&history) else {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.nothingToRetry"));
        };
        history.truncate(idx + 1);
    }
    // A session spans turns: an existing one is appended to, and without one it is created now and becomes the current session
    let log = {
        let mut current = state.agent.current_session.lock().unwrap();
        match current.as_deref() {
            Some(id) => SessionLog::open(&meta_dir, id).map_err(|e| e.to_string())?,
            None => {
                let log = SessionLog::create(&meta_dir).map_err(|e| e.to_string())?;
                *current = Some(log.id.clone());
                // A follow set before the first send lands in the fresh ledger too
                if let Some(f) = state.agent.follow.lock().unwrap().as_ref() {
                    log.append("follow", json!({ "path": &f.path, "auto": f.auto }));
                }
                log
            }
        }
    };
    let session_id = log.id.clone();
    if matches!(input, TurnInput::Retry) {
        log.append("retry", json!({}));
    }

    // The session-level snapshot: the system prompt prefix is frozen and
    // plan_confirm is fixed per session (a change takes effect from the next
    // session). Rebuilding the prompt between turns, or switching the tool table
    // mid-session, writes off the provider's prefix cache — on DeepSeek a hit is
    // roughly 30× cheaper.
    let (system_snapshot, plan_confirm) = {
        let mut setup = state.agent.session_setup.lock().unwrap();
        match setup.as_ref() {
            Some(s) => (s.system.clone(), s.plan_confirm),
            None => {
                let plan = settings.plan_confirm;
                let system = {
                    let guard = state.vault.lock().unwrap();
                    let vault = guard.as_ref().ok_or("no vault open")?;
                    vemry_agent::prompt::build_system_prompt(vault, plan, crate::i18n::current().tag())
                };
                *setup = Some(crate::agent_bridge::SessionSetup { system: system.clone(), plan_confirm: plan });
                (system, plan)
            }
        }
    };
    // Web search is read afresh every turn: the composer's web search switch decides, and
    // the endpoint must still be one that runs the search (a base_url moved away
    // from DeepSeek mid-session falls back to Chat Completions, where the switch is
    // disabled anyway). A turn that searches goes through the Anthropic-format line;
    // everything else stays on Chat Completions.
    let use_web_search = settings.web_search_active();
    let transport: Arc<dyn vemry_agent::ChatTransport> = if use_web_search {
        Arc::new(vemry_agent::AnthropicTransport::new())
    } else {
        Arc::new(HttpTransport::new())
    };
    let runner = AgentRunner {
        transport,
        gate: Arc::new(ChannelGate { pending: Arc::clone(&state.agent.pending.writes) }),
        plan_gate: Arc::new(crate::agent_bridge::ChannelPlanGate {
            pending: Arc::clone(&state.agent.pending.plans),
        }),
        actions_gate: Arc::new(crate::agent_bridge::ChannelActionsGate {
            pending: Arc::clone(&state.agent.pending.actions),
        }),
        read_gate: Arc::new(crate::agent_bridge::ChannelReadGate {
            app: app.clone(),
            pending: Arc::clone(&state.agent.pending.reads),
            auto_sources: Arc::clone(&state.agent.read_auto_sources_arc),
        }),
        vault: Arc::clone(&state.vault),
        sources: Arc::clone(&state.agent.sources),
        journal: Arc::clone(&state.agent.journal),
    };
    // The thinking level: an in-session override (passed by the frontend, and by a
    // retry too — a regenerate keeps the override) wins, otherwise the setting's
    // default applies; sent only to a DeepSeek endpoint (settings.reasoning_active
    // decides, the same gate as stream_options). It is a per-turn request parameter
    // and does not enter the frozen session snapshot.
    let effort = effort
        .filter(|e| settings::is_reasoning_effort(e))
        .unwrap_or_else(|| settings.reasoning_effort.clone());
    let (follow_note, follow_auto) = match state.agent.follow.lock().unwrap().as_ref() {
        Some(f) => (Some(f.path.clone()), f.auto),
        None => (None, true),
    };
    let config = RunnerConfig {
        max_iterations: 25,
        write_policy,
        format: settings.agent_format(),
        web_search: use_web_search,
        system_prompt: Some(system_snapshot),
        session_allow: Some(Arc::clone(&state.agent.session_grant)),
        reasoning_effort: settings.reasoning_active().then_some(effort),
        language: crate::i18n::current().tag().to_string(),
        plan_confirm,
        follow_note,
        follow_auto,
        ..Default::default()
    };

    let channel = on_event.clone();
    let app_for_events = app.clone();
    let emit = move |e: AgentEvent| {
        // A write the agent landed is a vault change like any other: the watcher
        // swallows it as a self-write, so it is broadcast from here (the open note
        // reloads, the index version moves, workflows see it)
        if let AgentEvent::VaultChanged { events } = &e {
            for ev in events {
                broadcast(&app_for_events, ev);
                if let VaultEvent::NoteModified { id } | VaultEvent::NoteAdded { id } = ev {
                    localize_agent_figures(&app_for_events, id.as_str());
                }
            }
        }
        let _ = channel.send(e);
    };

    let (message, page, refs, current_note) = match input {
        TurnInput::New { message, page, refs, current_note } => (Some(message), page, refs, current_note),
        TurnInput::Retry => (None, None, vec![], None),
    };
    let run_result = runner
        .run(&provider, &config, &mut history, message, page, refs, current_note, &emit, &state.agent.cancel, &log)
        .await;
    // Persist the model context at the end of the turn (written on an error or a cancellation too: it stays consistent with the in-memory state, so a restart can resume)
    write_history(&meta_dir, &session_id, &history);
    drop(history);
    // A hosted turn moved the balance: the snapshot is refreshed once the turn is
    // over, whatever its outcome (with the ETag, so an unchanged one is a 304), and
    // the account panel and the status bar follow through the account event
    if hosted {
        let app_for_refresh = app.clone();
        tokio::spawn(async move {
            let _ = crate::account::refresh(&app_for_refresh).await;
        });
    }

    // notes may have been written by the agent — let the UI refresh
    broadcast(app, &VaultEvent::IndexReady {
        note_count: state.vault.lock().unwrap().as_ref().map(|v| v.index().note_count()).unwrap_or(0),
    });

    // A relay error carries a code the user can act on (sign in, top up, wait):
    // rendered in their words, with a 401 clearing the token on the way (see
    // account::localize_relay_error). Everything else keeps its text as before.
    run_result.map_err(|e| {
        let text = e.to_string();
        if hosted {
            crate::account::localize_relay_error(app, &text).unwrap_or(text)
        } else {
            text
        }
    })?;
    Ok(session_id)
}

/// The first half of an edit-and-resend: drop the last user message and everything
/// after it (the frontend then sends the new text through the ordinary send). An
/// edit_resend marker is appended to the audit jsonl — on replay the frontend uses
/// it to drop the abandoned turn from the display, while the audit lines themselves
/// are untouched.
#[tauri::command]
pub fn agent_history_truncate(state: State<'_, AppState>) -> Result<(), String> {
    let mut history =
        state.agent.history.try_lock().map_err(|_| crate::i18n::t(crate::i18n::current(), "ipc.runningNoEditResend"))?;
    let Some(idx) = vemry_agent::compact::last_user_index(&history) else {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.sessionEmpty"));
    };
    // The abandoned turn's sources go with it (the replay applies the same rule from the marker line)
    if let Some(turn) = history[idx].turn.clone() {
        vemry_agent::sources::drop_turn(&mut state.agent.sources.lock().unwrap(), &turn);
    }
    history.truncate(idx);
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    if let Some(id) = state.agent.current_session.lock().unwrap().as_deref() {
        write_history(&meta_dir, id, &history);
    }
    append_session_line(&state, "edit_resend", json!({}))
}

/// The result of a fork: the new session id, plus whether the fork point had
/// already been swallowed into a summary by second-level compaction (when true, the
/// model context the new session carries is a synthesised summary covering more
/// than the fork point did, and the frontend tells the user).
#[derive(serde::Serialize)]
pub struct ForkResult {
    pub id: String,
    pub summarized_context: bool,
}

/// Fork a new session off the current one, before one of its user messages (the
/// research tree): the in-memory history is truncated and copied, the audit prefix
/// is copied, the sidecar is written, and the new session becomes current. The fork
/// point is located by turn id first (`turn`, the one the runner stamps on all
/// three layers) — the timeline, the audit jsonl and the model history each
/// counting user ordinals would fall out of step after compaction synthesises a
/// message or an edit leaves a stray line; when `turn` is None (an existing old
/// session with no ids) it falls back to counting ordinals.
#[tauri::command]
pub fn agent_session_fork(
    state: State<'_, AppState>,
    turn: Option<String>,
    before_user_turn: usize,
) -> Result<ForkResult, String> {
    let mut history =
        state.agent.history.try_lock().map_err(|_| crate::i18n::t(crate::i18n::current(), "ipc.runningNoFork"))?;
    let from_id =
        state.agent.current_session.lock().unwrap().clone().ok_or_else(|| crate::i18n::t(crate::i18n::current(), "ipc.noSessionToFork"))?;
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    let (cut, summarized_context) =
        vemry_agent::session::history_fork_cut(&history, turn.as_deref(), before_user_turn);
    let forked: Vec<vemry_agent::types::ChatMessage> = history[..cut].to_vec();
    let log = fork_session(&meta_dir, &from_id, turn.as_deref(), before_user_turn)
        .map_err(|e| e.to_string())?;
    write_history(&meta_dir, &log.id, &forked);
    *history = forked;
    *state.agent.current_session.lock().unwrap() = Some(log.id.clone());
    state.agent.reset_session_scope();
    state.agent.load_session(&meta_dir, &log.id); // the copied prefix carries the consents given before the cut
    Ok(ForkResult { id: log.id, summarized_context })
}

/// Append one line to the current session's ledger. "No session yet" is not an
/// error anywhere this is used: a line about a session that has not been created
/// has nowhere to go, and neither has one about a session file that will not open.
fn append_session_line(state: &State<'_, AppState>, kind: &str, payload: serde_json::Value) -> Result<(), String> {
    let meta_dir = with_vault(state, |v| Ok(v.meta_dir()))?;
    if let Some(id) = state.agent.current_session.lock().unwrap().as_deref() {
        if let Ok(log) = SessionLog::open(&meta_dir, id) {
            log.append(kind, payload);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn agent_permission_respond(
    state: State<'_, AppState>,
    request_id: String,
    decision: PermissionDecision,
) -> Result<(), String> {
    if matches!(decision, PermissionDecision::AllowAlways) {
        persist_allow_always(&state);
    }
    resolve(&state.agent.pending.writes, "permission", &request_id, decision)
}

/// Handing a plan-gate decision back (propose_plan): resolve the matching oneshot, and the loop continues within the same turn.
#[tauri::command]
pub fn agent_plan_respond(
    state: State<'_, AppState>,
    request_id: String,
    decision: vemry_agent::PlanDecision,
) -> Result<(), String> {
    resolve(&state.agent.pending.plans, "plan", &request_id, decision)
}

/// Undo every write one turn landed (see vemry_agent::undo): notes get their previous
/// text back when they still hold what the write left, created notes go to the
/// trash, moves move back, rewrites are restored; anything edited since is skipped.
/// Refused while the agent runs (the journal is being appended to). An `undo` marker
/// goes into the jsonl (counts only, never the texts). Every note put back is
/// announced with its own vault event: the watcher swallows the undo's writes as the
/// app's own, and IndexReady alone never reloads an open buffer — which would keep
/// the agent's text and autosave it straight back.
#[tauri::command]
pub fn agent_undo_turn(app: AppHandle, state: State<'_, AppState>, turn: String) -> Result<vemry_agent::UndoReport, String> {
    if state.agent.running.load(Ordering::SeqCst) {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.agentRunning"));
    }
    let report = {
        let mut guard = state.vault.lock().unwrap();
        let vault = guard.as_mut().ok_or("no vault open")?;
        let mut journal = state.agent.journal.lock().unwrap();
        vemry_agent::undo::undo_turn(vault, &mut journal, &turn)
    };
    append_session_line(&state, "undo", json!({ "turn": &turn, "reverted": report.reverted, "skipped": report.skipped }))?;
    for change in &report.changes {
        broadcast(&app, &change.event());
    }
    broadcast(&app, &VaultEvent::IndexReady {
        note_count: state.vault.lock().unwrap().as_ref().map(|v| v.index().note_count()).unwrap_or(0),
    });
    Ok(report)
}

/// Handing a read-gate answer back (open_for_reading): which URLs the user allowed
/// (the gate then fetches them), and whether the session's source-table URLs may be
/// opened without asking from now on.
#[tauri::command]
pub fn agent_read_respond(
    state: State<'_, AppState>,
    request_id: String,
    answer: crate::agent_bridge::ReadAnswer,
) -> Result<(), String> {
    resolve(&state.agent.pending.reads, "read", &request_id, answer)
}

/// Handing an action-list decision back (propose_actions): resolve the matching oneshot, and the loop continues within the same turn.
#[tauri::command]
pub fn agent_actions_respond(
    state: State<'_, AppState>,
    request_id: String,
    decision: vemry_agent::ActionsDecision,
) -> Result<(), String> {
    resolve(&state.agent.pending.actions, "actions", &request_id, decision)
}

/// Follow a note (or stop, with path=None): the session keeps its findings there and
/// local writes on it (append / frontmatter / edit) pass the gate without a dialog —
/// the follow is the user's grant for that one note; `auto=false` keeps the follow
/// (the model is told which note to work in) but asks for every write again. Appended
/// to the current session's jsonl as a `follow` line (the last one wins on replay); a
/// session that has not sent yet gets the line when its ledger is created. Allowed
/// while the agent runs, but the running turn keeps the follow it started with (the
/// runner snapshots it into its config at the head of the turn): a change made
/// mid-turn applies from the next turn.
#[tauri::command]
pub fn agent_follow_set(state: State<'_, AppState>, path: Option<String>, auto: bool) -> Result<(), String> {
    let path = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());
    if let Some(p) = &path {
        // A private note is not followed: the runner would filter it out of the context anyway, and the chip must not pretend
        let private = with_vault(&state, |v| Ok(v.is_private_rel(p, false)))?;
        if private {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.followPrivate"));
        }
    }
    *state.agent.follow.lock().unwrap() = path.clone().map(|p| vemry_agent::Follow { path: p, auto });
    append_session_line(&state, "follow", json!({ "path": path, "auto": auto }))
}

#[tauri::command]
pub fn agent_cancel(state: State<'_, AppState>) {
    state.agent.cancel.store(true, Ordering::SeqCst);
    state.agent.pending.deny_all();
}

/// Whether the agent is running a turn. Cancelling only sets a flag, and the turn
/// really stops at the next streaming chunk boundary; after the chat window closes,
/// the main window polls this before taking over so it waits for the session jsonl
/// to be finished rather than reading a half-written session.
#[tauri::command]
pub fn agent_is_running(state: State<'_, AppState>) -> bool {
    state.agent.running.load(Ordering::SeqCst)
}

/// New session: clear the model context; the id is allocated only on the first send,
/// so an empty session never reaches disk.
/// try_lock: while the agent is running the runner holds history, so this refuses
/// rather than queueing.
#[tauri::command]
pub fn agent_session_new(state: State<'_, AppState>) -> Result<(), String> {
    let mut history =
        state.agent.history.try_lock().map_err(|_| crate::i18n::t(crate::i18n::current(), "ipc.runningNoNewSession"))?;
    history.clear();
    *state.agent.current_session.lock().unwrap() = None;
    state.agent.reset_session_scope();
    state.agent.clear_session_data();
    Ok(())
}

#[tauri::command]
pub fn agent_session_list(state: State<'_, AppState>) -> Result<Vec<SessionMeta>, String> {
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    Ok(state.agent.session_meta.lock().unwrap().list(&meta_dir))
}

/// Switch sessions: sidecar → model context (an existing old file with no sidecar →
/// an empty context, while the record can still be read); returns the full jsonl for
/// the frontend to rebuild the timeline from.
#[tauri::command]
pub fn agent_session_select(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let (root, meta_dir) = {
        let guard = state.vault.lock().unwrap();
        let vault = guard.as_ref().ok_or("no vault open")?;
        (vault.root().to_path_buf(), vault.meta_dir())
    };
    let jsonl = read_session(&meta_dir, &id).ok_or_else(|| "session not found".to_string())?;
    let mut history =
        state.agent.history.try_lock().map_err(|_| crate::i18n::t(crate::i18n::current(), "ipc.runningNoSwitch"))?;
    *history = read_history(&meta_dir, &id).unwrap_or_default();
    *state.agent.history_vault.lock().unwrap() = Some(root);
    state.agent.reset_session_scope();
    state.agent.load_session(&meta_dir, &id);
    *state.agent.current_session.lock().unwrap() = Some(id);
    Ok(jsonl)
}

/// Rename a session by hand: a pure jsonl append (one whole line written with
/// O_APPEND, the last title override winning) that does not touch the history
/// sidecar — it takes no history lock and is allowed while the agent is running.
#[tauri::command]
pub fn agent_session_rename(state: State<'_, AppState>, id: String, title: String) -> Result<(), String> {
    with_vault(&state, |v| rename_session(&v.meta_dir(), &id, &title))
}

/// Delete a session (jsonl plus sidecar; the audit record goes with it — the
/// frontend must confirm).
/// Deleting the current session is equivalent to starting a new one.
#[tauri::command]
pub fn agent_session_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let is_current = state.agent.current_session.lock().unwrap().as_deref() == Some(id.as_str());
    if is_current {
        let mut history =
            state.agent.history.try_lock().map_err(|_| crate::i18n::t(crate::i18n::current(), "ipc.runningNoDelete"))?;
        history.clear();
        *state.agent.current_session.lock().unwrap() = None;
        state.agent.reset_session_scope();
        state.agent.clear_session_data();
    }
    with_vault(&state, |v| delete_session(&v.meta_dir(), &id))
}

/// The current session id (None = a new session awaiting its first send). The
/// detached chat window at startup, and the main window taking over afterwards, use
/// it to find which jsonl to rebuild the timeline from — it does not switch the
/// model context, which is what makes it different from select.
#[tauri::command]
pub fn agent_session_current(state: State<'_, AppState>) -> Option<String> {
    state.agent.current_session.lock().unwrap().clone()
}

/// Read-only fetching for the replay view; the list and the research panel share
/// agent_session_list (since v0.7 the two session IPCs are collected together, and
/// agent_session_select is kept away from replay because it switches the model
/// context).
#[tauri::command]
pub fn session_read(state: State<'_, AppState>, id: String) -> Result<String, String> {
    with_vault(&state, |v| {
        read_session(&v.meta_dir(), &id).ok_or_else(|| "session not found".into())
    })
}

/// The quick-memory convention of a chat input starting with `#`: one bullet lands
/// in VEMRY.md's conventions section (scaffolding the file or the section when
/// either is missing). A deliberate user action, so it does not pass the AI
/// permission gate.
#[tauri::command]
pub fn memory_append(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.contentEmpty"));
    }
    let lang = crate::i18n::resolve(settings::load(&app).language);
    let (header, section) =
        (crate::i18n::t(lang, "memoryFile.header"), crate::i18n::t(lang, "memoryFile.section"));
    let event = with_vault_mut(&state, |v| {
        v.memory_append(&text, &header, &section).map_err(crate::i18n::err)
    })?;
    broadcast(&app, &event);
    Ok(())
}

#[tauri::command]
pub fn init_memory_file(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let event = with_vault_mut(&state, |v| {
        let id = v.note_id("VEMRY.md").map_err(crate::i18n::err)?;
        if v.read_note(&id).is_ok() {
            return Err(crate::i18n::t(crate::i18n::current(), "ipc.memoryExists"));
        }
        v.write_note(&id, &memory_template(&app)).map_err(crate::i18n::err)
    })?;
    remember_open_vault(&app, &state);
    broadcast(&app, &event);
    Ok("VEMRY.md".into())
}

/// One row of the example block types catalogue in Settings: installed = a file of
/// that name is already in `.vemry/types/` (the user's, whether or not its text is ours).
#[derive(Serialize)]
pub struct TypeExample {
    pub name: String,
    pub description: String,
    pub installed: bool,
}

/// The example block types Settings offers.
#[tauri::command]
pub fn type_examples_list(state: State<'_, AppState>) -> Result<Vec<TypeExample>, String> {
    let dir = with_vault(&state, |v| Ok(v.root().join(vemry_core::vault::TYPES_DIR)))?;
    Ok(crate::type_templates::type_examples()
        .iter()
        .map(|e| TypeExample { name: e.name.clone(), description: e.description.clone(), installed: dir.join(e.file).exists() })
        .collect())
}

/// Install one example block type into `.vemry/types/` (never overwriting); the
/// editor's menus and the agent's next session pick it up (TypesChanged is broadcast).
#[tauri::command]
pub fn type_install_example(app: AppHandle, state: State<'_, AppState>, name: String) -> Result<Scaffolded, String> {
    let example = crate::type_templates::example_type(&name).ok_or_else(|| format!("unknown example type: {name}"))?;
    let rel = format!("{}/{}", vemry_core::vault::TYPES_DIR, example.file);
    let created = scaffold_meta(&state, &rel, example.body)?;
    if created {
        broadcast(&app, &VaultEvent::TypesChanged);
    }
    Ok(Scaffolded { path: rel, created })
}

/// One row of the skills list in Settings: the union of the built-ins and the vault's
/// own files, disabled ones included — the management view differs from the injection
/// view, and `list_skills` shows the agent only what is enabled. `source` says which
/// layer the row is (`builtin` / `vault` / `override`); `outdated` flags a customised
/// copy the built-in has moved past since it was made.
#[derive(Serialize)]
pub struct SkillEntry {
    pub file: String,
    pub name: String,
    pub description: String,
    pub disabled: bool,
    pub source: &'static str,
    pub version: u64,
    pub outdated: bool,
    /// The words the `/` picker matches besides the name and the description.
    pub triggers: Vec<String>,
}

#[tauri::command]
pub fn skills_list(state: State<'_, AppState>) -> Result<Vec<SkillEntry>, String> {
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    // The directory scan, the built-in catalogue and the frontmatter parsing live in
    // vemry-agent's skills.rs (the same implementation as the agent's injection
    // view); this only maps the IPC shape
    Ok(vemry_agent::skills::list_skills_all(&meta_dir)
        .into_iter()
        .map(|s| SkillEntry {
            file: s.file,
            name: s.name,
            description: s.description,
            disabled: s.disabled,
            source: s.source.as_str(),
            version: s.version,
            outdated: s.outdated,
            triggers: s.triggers,
        })
        .collect())
}

/// The vault-relative path of a skill file, the one spelling the four commands share.
fn skill_rel_path(file: &str) -> String {
    format!(".vemry/{}/{file}", vemry_agent::skills::SKILLS_SUBDIR)
}

/// The listed skill of that name — the listing is what refuses a symlink, so a file
/// name taken from it is safe to write.
fn skill_entry(meta_dir: &std::path::Path, name: &str) -> Result<vemry_agent::skills::SkillFileEntry, String> {
    vemry_agent::skills::list_skills_all(meta_dir)
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| crate::i18n::t(crate::i18n::current(), "ipc.skillNotFound"))
}

/// Enable or disable one skill by its listed name. A vault file has the `disabled:`
/// line of its frontmatter rewritten in place (through `Vault::write_meta`, like an
/// editor save); a built-in has no file, so its name goes into (or out of) the vault
/// config's `skills_disabled` list. The skill list is part of the prompt snapshot, so
/// the snapshot is dropped: the switch applies from the next turn.
#[tauri::command]
pub fn skill_set_enabled(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), String> {
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    let entry = skill_entry(&meta_dir, &name)?;
    if entry.source == vemry_agent::skills::SkillSource::Builtin {
        let key = vemry_agent::skills::DISABLED_BUILTINS_KEY;
        with_vault(&state, |v| {
            let mut off = vemry_agent::skills::disabled_names(vault_config_get(v, key).as_ref());
            if enabled {
                off.retain(|n| n != &name);
            } else if !off.contains(&name) {
                off.push(name.clone());
            }
            vault_config_set(v, key, json!(off))
        })?;
    } else {
        with_vault_mut(&state, |v| {
            let id = v.note_id(&skill_rel_path(&entry.file)).map_err(crate::i18n::err)?;
            let raw = v.read_note(&id).map_err(crate::i18n::err)?;
            v.write_meta(&id, &vemry_core::set_frontmatter_disabled(&raw, !enabled)).map_err(crate::i18n::err)
        })?;
    }
    state.agent.invalidate_prompt_snapshot();
    Ok(())
}

/// A customised copy of a built-in: the built-in's file written into `.vemry/skills/`
/// (never over one that is there), which shadows the built-in by name from then on
/// and is the user's to edit. A name the vault already overrides under some other file
/// name hands that file back instead of creating a second one; a file already under the
/// built-in's file name that lists as some other skill is an error naming it, not the
/// copy (a copy needs that file name, and the file there is not this skill). The
/// frontend opens the result.
#[tauri::command]
pub fn skill_customize(state: State<'_, AppState>, name: String) -> Result<Scaffolded, String> {
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    let entry = skill_entry(&meta_dir, &name)?;
    if entry.source != vemry_agent::skills::SkillSource::Builtin {
        return Ok(Scaffolded { path: skill_rel_path(&entry.file), created: false });
    }
    let builtin = vemry_agent::skills::builtin_skill(&name)
        .ok_or_else(|| crate::i18n::t(crate::i18n::current(), "ipc.skillNotFound"))?;
    // The listing resolved the name to the built-in, so no vault file lists under it:
    // whatever is at the copy's path belongs to another skill.
    if meta_dir.join(vemry_agent::skills::SKILLS_SUBDIR).join(builtin.file).exists() {
        return Err(crate::i18n::tf(crate::i18n::current(), "ipc.skillFileTaken", &[("file", builtin.file)]));
    }
    let created = scaffold_meta(&state, &skill_rel_path(builtin.file), builtin.text)?;
    state.agent.invalidate_prompt_snapshot();
    Ok(Scaffolded { path: skill_rel_path(builtin.file), created })
}

/// Back to the built-in: the customised copy goes to the OS trash (recoverable, like
/// every delete), so the built-in of that name shows again from the next turn.
#[tauri::command]
pub fn skill_restore(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    let entry = skill_entry(&meta_dir, &name)?;
    if entry.source != vemry_agent::skills::SkillSource::Override {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.skillNotFound"));
    }
    with_vault_mut(&state, |v| v.delete_path(&skill_rel_path(&entry.file)).map_err(crate::i18n::err))?;
    state.agent.invalidate_prompt_snapshot();
    Ok(())
}

/// A new vault skill from the skeleton every built-in follows (`.vemry/skills/<stem>.md`,
/// the stem sanitised the way a note title is). The skeleton lists under `name`, so the
/// name decides: a built-in's name is refused (an empty copy would shadow it — customising
/// is how a built-in is changed), a vault skill already listed under it is handed back
/// from whatever file it lives in, and a file at the skeleton's path that lists as some
/// other skill is an error naming it. The frontend opens the result.
#[tauri::command]
pub fn skill_create(state: State<'_, AppState>, name: String) -> Result<Scaffolded, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.skillNameEmpty"));
    }
    let stem = vemry_core::sanitize_filename(name);
    let file = format!("{stem}.md");
    let meta_dir = with_vault(&state, |v| Ok(v.meta_dir()))?;
    match vemry_agent::skills::list_skills_all(&meta_dir).into_iter().find(|s| s.name == name) {
        Some(s) if s.source == vemry_agent::skills::SkillSource::Builtin => {
            return Err(crate::i18n::tf(crate::i18n::current(), "ipc.skillNameBuiltin", &[("name", name)]));
        }
        Some(s) => return Ok(Scaffolded { path: skill_rel_path(&s.file), created: false }),
        None => {}
    }
    if meta_dir.join(vemry_agent::skills::SKILLS_SUBDIR).join(&file).exists() {
        return Err(crate::i18n::tf(crate::i18n::current(), "ipc.skillFileTaken", &[("file", &file)]));
    }
    let created = scaffold_meta(&state, &skill_rel_path(&file), &vemry_agent::skills::skill_skeleton(name))?;
    state.agent.invalidate_prompt_snapshot();
    Ok(Scaffolded { path: skill_rel_path(&file), created })
}

/// Vault-relative paths currently in the privacy zone — drives the badges in the
/// file tree. Human-only: the agent never sees this list.
#[tauri::command]
pub fn privacy_list(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| {
        let mut out: Vec<String> = v
            .index()
            .summaries()
            .into_iter()
            .filter(|s| v.is_private(&s.id))
            .map(|s| s.id.to_string())
            .collect();
        out.extend(v.list_folders().into_iter().filter(|f| v.is_private_rel(f, true)));
        out.extend(v.list_files().into_iter().filter(|f| v.is_private_rel(f, false)));
        out.sort();
        out.dedup();
        Ok(out)
    })
}

/// Mark a path as private. Notes get frontmatter `private: true` (travels
/// with the file); folders and non-note files get a `.vemry/privacy` rule.
#[tauri::command]
pub fn privacy_mark(app: AppHandle, state: State<'_, AppState>, path: String, kind: String) -> Result<(), String> {
    let note_event = with_vault_mut(&state, |v| match kind.as_str() {
        "note" => {
            let id = v.note_id(&path).map_err(crate::i18n::err)?;
            v.set_note_private(&id, true).map(Some).map_err(crate::i18n::err)
        }
        "folder" => v.add_privacy_rule(&format!("{}/", path.trim_end_matches('/'))).map(|_| None).map_err(crate::i18n::err),
        _ => v.add_privacy_rule(&path).map(|_| None).map_err(crate::i18n::err),
    })?;
    // set_note_private writes through write_note with watcher self-write
    // suppression — forward its NoteModified so an open editor reloads the
    // new frontmatter instead of clobbering it on the next save
    if let Some(ev) = note_event {
        broadcast(&app, &ev);
    }
    // The frozen session-level system prompt carries the vault overview and
    // VEMRY.md — a change to the privacy zones invalidates the snapshot at once, or
    // newly marked content would keep going to the model with the old prompt until
    // the session ended
    state.agent.invalidate_prompt_snapshot();
    broadcast(&app, &VaultEvent::PrivacyChanged);
    Ok(())
}

/// Undo `privacy_mark`. Only the exact rule line is removed — a path still
/// covered by a broader rule stays private (and keeps its badge).
#[tauri::command]
pub fn privacy_unmark(app: AppHandle, state: State<'_, AppState>, path: String, kind: String) -> Result<(), String> {
    let note_event = with_vault_mut(&state, |v| match kind.as_str() {
        "note" => {
            let id = v.note_id(&path).map_err(crate::i18n::err)?;
            v.set_note_private(&id, false).map(Some).map_err(crate::i18n::err)
        }
        "folder" => v.remove_privacy_rule(&format!("{}/", path.trim_end_matches('/'))).map(|_| None).map_err(crate::i18n::err),
        _ => v.remove_privacy_rule(&path).map(|_| None).map_err(crate::i18n::err),
    })?;
    if let Some(ev) = note_event {
        broadcast(&app, &ev);
    }
    state.agent.invalidate_prompt_snapshot();
    broadcast(&app, &VaultEvent::PrivacyChanged);
    Ok(())
}

// ---- user scripts (v0.3 programmability) ----

fn script_host(state: &State<'_, AppState>) -> Result<vemry_script::ScriptHost, String> {
    state.scripts.lock().unwrap().clone().ok_or_else(|| "no vault open".to_string())
}

/// Snapshot of loaded scripts (commands, triggers, errors) — non-blocking.
#[tauri::command]
pub fn scripts_list(state: State<'_, AppState>) -> Vec<vemry_script::ScriptInfo> {
    state.scripts.lock().unwrap().as_ref().map(|h| h.list()).unwrap_or_default()
}

/// Explicit reload button — waits for completion so the caller can re-list.
/// Propagates a timeout instead of letting the UI present stale state as
/// freshly reloaded.
#[tauri::command]
pub async fn scripts_reload(state: State<'_, AppState>) -> Result<(), String> {
    let host = script_host(&state)?;
    tokio::task::spawn_blocking(move || host.reload_blocking()).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn script_set_enabled(state: State<'_, AppState>, name: String, enabled: bool) -> Result<(), String> {
    let host = script_host(&state)?;
    tokio::task::spawn_blocking(move || host.set_enabled(&name, enabled)).await.map_err(|e| e.to_string())?
}

/// Fire a script command from the palette. Fire-and-forget: outcomes surface
/// as `script://event` toasts/errors, the palette never blocks.
#[tauri::command]
pub fn script_command_run(
    state: State<'_, AppState>,
    script: String,
    command: String,
    current_note: Option<String>,
) -> Result<(), String> {
    script_host(&state)?.run_command(&script, &command, current_note)
}

/// The user's answer to a script write-grant dialog.
#[tauri::command]
pub fn script_grant_respond(state: State<'_, AppState>, request_id: String, allow: bool) -> Result<(), String> {
    state.script_bridge.resolve_grant(&request_id, allow)
}

/// Withdraw a previously granted write permission — the next write asks again.
#[tauri::command]
pub async fn script_grant_revoke(state: State<'_, AppState>, name: String) -> Result<(), String> {
    let host = script_host(&state)?;
    tokio::task::spawn_blocking(move || host.revoke_write_grant(&name)).await.map_err(|e| e.to_string())?
}

/// The user's answer to a `vemry.prompt()` dialog (`None` = cancelled).
#[tauri::command]
pub fn script_prompt_respond(state: State<'_, AppState>, request_id: String, value: Option<String>) -> Result<(), String> {
    state.script_bridge.resolve_prompt(&request_id, value)
}

/// Run a note's `js` code block in the one-shot read-only sandbox. Strictly
/// user-clicked (the run button) — there is no auto-run path. `params` are the
/// current values of the block's lab controls on a re-run (the user moved a
/// slider of a block they already ran); `None` on the first run.
#[tauri::command]
pub async fn script_eval_block(
    state: State<'_, AppState>,
    code: String,
    current_note: Option<String>,
    params: Option<serde_json::Value>,
) -> Result<vemry_script::BlockEvalResult, String> {
    let host = script_host(&state)?;
    tokio::task::spawn_blocking(move || host.eval_block(&code, current_note, params)).await.map_err(|e| e.to_string())
}

/// Raw rule lines of `.vemry/privacy` — the glob half of the privacy zone
/// (frontmatter-marked notes are not in this list).
#[tauri::command]
pub fn privacy_rules(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    with_vault(&state, |v| Ok(v.privacy().rules().to_vec()))
}

/// Append a raw rule line (gitignore glob syntax, `!` negations included).
#[tauri::command]
pub fn privacy_rule_add(app: AppHandle, state: State<'_, AppState>, line: String) -> Result<(), String> {
    with_vault_mut(&state, |v| v.add_privacy_rule(&line).map_err(crate::i18n::err))?;
    state.agent.invalidate_prompt_snapshot();
    broadcast(&app, &VaultEvent::PrivacyChanged);
    Ok(())
}

/// Remove the exact rule line. Unlike `privacy_unmark` this takes the raw
/// line as written in the file — no folder `/` suffix normalization.
#[tauri::command]
pub fn privacy_rule_remove(app: AppHandle, state: State<'_, AppState>, line: String) -> Result<(), String> {
    with_vault_mut(&state, |v| v.remove_privacy_rule(&line).map_err(crate::i18n::err))?;
    state.agent.invalidate_prompt_snapshot();
    broadcast(&app, &VaultEvent::PrivacyChanged);
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::decode_path_header;

    /// The "empty directory" branch of the recent-history check: hidden entries do not count, and unreadable counts as not empty.
    #[test]
    fn dir_effectively_empty_ignores_hidden_entries() {
        let base = std::env::temp_dir().join(format!("vemry-empty-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert!(super::dir_effectively_empty(&base));
        std::fs::write(base.join(".DS_Store"), b"x").unwrap();
        std::fs::write(base.join("Desktop.ini"), b"x").unwrap();
        std::fs::write(base.join("Thumbs.db"), b"x").unwrap();
        assert!(super::dir_effectively_empty(&base));
        std::fs::write(base.join("readme.txt"), b"x").unwrap();
        assert!(!super::dir_effectively_empty(&base));
        assert!(!super::dir_effectively_empty(&base.join("missing")));
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Simulate JS `encodeURIComponent` character by character (the reserved set
    /// A-Za-z0-9 -_.!~*'()), pinning the frontend's encoding contract into a
    /// round-trip test.
    fn encode_uri_component(s: &str) -> String {
        let mut out = String::new();
        for b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(*b as char),
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(*b as char),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    #[test]
    fn chinese_filename_round_trips() {
        let path = "attachments/我的笔记/年度 报告 v2.pdf";
        assert_eq!(decode_path_header(&encode_uri_component(path)).unwrap(), path);
    }

    #[test]
    fn emoji_filename_round_trips() {
        let path = "attachments/inbox/📄 计划🚀.pdf";
        assert_eq!(decode_path_header(&encode_uri_component(path)).unwrap(), path);
    }

    #[test]
    fn literal_percent_round_trips() {
        // A literal % in a file name has to be encoded as %25 by the frontend to round-trip losslessly
        let path = "attachments/note/进度50%.png";
        assert_eq!(decode_path_header(&encode_uri_component(path)).unwrap(), path);
    }

    #[test]
    fn plain_ascii_passes_through_unchanged() {
        // Pure ASCII with no reserved character: encoding is the identity, and so is decoding (the old behaviour is unchanged)
        assert_eq!(decode_path_header("img.png").unwrap(), "img.png");
        assert_eq!(
            decode_path_header(&encode_uri_component("attachments/note/img.png")).unwrap(),
            "attachments/note/img.png"
        );
    }

    #[test]
    fn invalid_utf8_percent_sequence_errors() {
        // %FF alone is not valid UTF-8 — an error rather than silently producing a mangled path
        assert!(decode_path_header("bad%FF.pdf").is_err());
    }
}
