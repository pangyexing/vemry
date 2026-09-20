//! Pure logic for the attachment-bucket migration: the old bucket
//! `attachments/<stem>/` (named after the note's file name alone, so notes
//! sharing a stem share a bucket and ownership cannot be decided) → the new
//! bucket `attachments/<note path minus .md>/` (a note path is unique, so the
//! bucket is too). Ownership, the new bucket path and link rewriting live here;
//! the filesystem choreography is a thin method in vault.rs
//! (migrate_attachment_buckets).
//! Migration principle: **only buckets with a unique owner move** — mixed
//! buckets (several notes with one stem) and orphans (no note at all) cannot be
//! split or attributed automatically, so they stay put and land in the report.
//! Better to leave a bucket alone than to move it to the wrong place.

use serde::Serialize;

#[derive(Debug, Default, Serialize)]
pub struct MigrationReport {
    /// (old bucket rel path, new bucket rel path); a preview under dry_run
    pub moved: Vec<(String, String)>,
    /// How many notes had links rewritten
    pub rewritten_notes: usize,
    /// Mixed buckets (several notes share the stem): kept in place, the user
    /// is told to sort them out by hand
    pub mixed: Vec<String>,
    /// Orphan buckets with no owning note (deleted or renamed): kept in place
    pub orphaned: Vec<String>,
    /// The new bucket path is already taken: skipped (no per-file merge, so
    /// the report stays legible)
    pub conflicts: Vec<String>,
}

/// The ownership verdict for a stem.
#[derive(Debug, PartialEq, Eq)]
pub enum Owner<'a> {
    /// Exactly one note matches → migratable
    Unique(&'a str),
    /// No note at all (orphan bucket)
    None,
    /// Several notes (mixed bucket, cannot be split automatically)
    Mixed,
}

/// The single note owning a stem: the whole vault's basenames (minus .md)
/// compared case-insensitively — matching the default semantics of APFS/NTFS,
/// where `Note.md` and the bucket `note/` are the same name.
pub fn owner_of<'a>(stem: &str, note_ids: &'a [String]) -> Owner<'a> {
    let lower = stem.to_lowercase();
    let mut hit: Option<&str> = None;
    for id in note_ids {
        let base = id.rsplit('/').next().unwrap_or(id);
        let Some(s) = base.strip_suffix(".md") else { continue };
        if s.to_lowercase() == lower {
            if hit.is_some() {
                return Owner::Mixed;
            }
            hit = Some(id.as_str());
        }
    }
    hit.map_or(Owner::None, Owner::Unique)
}

/// A note's new attachment bucket: `attachments/<path minus .md>`. Root-level
/// notes keep the old path (which is why the migration only really touches
/// notes in subfolders). The frontend's noteBucketOf mirrors this.
pub fn bucket_for(note_id: &str) -> String {
    format!("attachments/{}", note_id.strip_suffix(".md").unwrap_or(note_id))
}

/// Whether a vault path lies in an attachments folder: any segment named `attachments`,
/// so the current buckets (`attachments/<note path>/`), the pre-v0.9 ones and a folder's
/// own `attachments/` all count. Such a folder holds a note's files — images, PDFs, an
/// exported bundle — and is never a home for a note's content, even when a `.md` sits
/// there: propose_actions refuses a target in one, and the action card's candidate list
/// leaves the notes found there out. The frontend's inAttachments mirrors this.
pub fn in_attachments(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "attachments")
}

/// The URL-encoded form of a link target. **Mirrors encodeTarget in the
/// frontend's Editor.svelte**: only `%`, space, `(` and `)` are escaped (the
/// minimum Markdown link syntax needs); `/` and non-ASCII characters are left
/// alone — change one side and you must change the other.
pub fn encode_target(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for c in rel.chars() {
        match c {
            '%' => out.push_str("%25"),
            ' ' => out.push_str("%20"),
            '(' => out.push_str("%28"),
            ')' => out.push_str("%29"),
            _ => out.push(c),
        }
    }
    out
}

/// Rewrite one note's links from the old bucket to the new one. A literal
/// prefix substitution, done only inside link syntax — `](old/` (Markdown links
/// and embeds, in both the raw and the encodeTarget-encoded form) and `[[old/`
/// (wikilinks, never encoded); code fences are skipped.
/// A bare-filename `![[img.png]]` is left alone: it resolves by shortest path,
/// the file name has not changed, so it still resolves after the move. None =
/// nothing changed. The walk is `relink::replace_outside_fences`, shared with the
/// note and file move planners.
pub fn rewrite_links(content: &str, old_bucket: &str, new_bucket: &str) -> Option<String> {
    let pairs = [
        (format!("]({old_bucket}/"), format!("]({new_bucket}/")),
        (format!("]({}/", encode_target(old_bucket)), format!("]({}/", encode_target(new_bucket))),
        (format!("[[{old_bucket}/"), format!("[[{new_bucket}/")),
        (format!("<{old_bucket}/"), format!("<{new_bucket}/")),
    ];
    let (out, count) = crate::relink::replace_outside_fences(content, &pairs);
    (count > 0).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn owner_unique_none_and_mixed() {
        let notes = ids(&["a/index.md", "b/index.md", "projects/plan.md", "Top.md"]);
        assert_eq!(owner_of("plan", &notes), Owner::Unique("projects/plan.md"));
        assert_eq!(owner_of("index", &notes), Owner::Mixed); // mixed bucket
        assert_eq!(owner_of("ghost", &notes), Owner::None); // orphan
        // case-insensitive (APFS semantics)
        assert_eq!(owner_of("top", &notes), Owner::Unique("Top.md"));
    }

    /// A path in an attachments folder, at the root or below a folder, is in one; a note
    /// merely named after the word is not.
    #[test]
    fn in_attachments_is_by_segment() {
        assert!(in_attachments("attachments/plan/a.png"));
        assert!(in_attachments("attachments/site-v1/research/rules.md"));
        assert!(in_attachments("projects/attachments/x.md"));
        assert!(in_attachments("attachments"));
        assert!(!in_attachments("attachments-list.md"));
        assert!(!in_attachments("notes/my attachments.md"));
        assert!(!in_attachments("inbox.md"));
    }

    #[test]
    fn bucket_for_mirrors_note_path() {
        assert_eq!(bucket_for("projects/a/index.md"), "attachments/projects/a/index");
        // A root-level note keeps the old bucket path → the caller skips it, nothing moves
        assert_eq!(bucket_for("inbox.md"), "attachments/inbox");
    }

    #[test]
    fn rewrite_md_links_and_embeds() {
        let src = "![img](attachments/plan/a.png)\n[f](attachments/plan/b (1).pdf)\n";
        let out = rewrite_links(src, "attachments/plan", "attachments/projects/plan").unwrap();
        assert_eq!(
            out,
            "![img](attachments/projects/plan/a.png)\n[f](attachments/projects/plan/b (1).pdf)\n"
        );
    }

    #[test]
    fn rewrite_encoded_form() {
        // When the bucket name contains a space, the Markdown link holds the encodeTarget form
        let src = "![x](attachments/my%20note/a.png)\n";
        let out = rewrite_links(src, "attachments/my note", "attachments/dir/my note").unwrap();
        assert_eq!(out, "![x](attachments/dir/my%20note/a.png)\n");
    }

    #[test]
    fn rewrite_wikilink_form() {
        let src = "![[attachments/plan/a.png]] and [[attachments/plan/b.png|alias]]\n";
        let out = rewrite_links(src, "attachments/plan", "attachments/p/plan").unwrap();
        assert_eq!(out, "![[attachments/p/plan/a.png]] and [[attachments/p/plan/b.png|alias]]\n");
    }

    #[test]
    fn skips_code_fences() {
        let src = "```\n![x](attachments/plan/a.png)\n```\n![y](attachments/plan/b.png)\n";
        let out = rewrite_links(src, "attachments/plan", "attachments/p/plan").unwrap();
        assert_eq!(out, "```\n![x](attachments/plan/a.png)\n```\n![y](attachments/p/plan/b.png)\n");
    }

    #[test]
    fn fence_state_matches_parser() {
        // A ``` line inside a ~~~ block must not close the fence; only a longer closer does
        let src = "~~~\n```\n![x](attachments/plan/a.png)\n~~~\n![y](attachments/plan/b.png)\n";
        let out = rewrite_links(src, "attachments/plan", "attachments/p/plan").unwrap();
        assert_eq!(
            out,
            "~~~\n```\n![x](attachments/plan/a.png)\n~~~\n![y](attachments/p/plan/b.png)\n"
        );
    }

    #[test]
    fn bare_filenames_untouched() {
        // Bare-filename embeds resolve by shortest path, so they are out of scope
        assert_eq!(rewrite_links("![[a.png]]\n", "attachments/plan", "attachments/p/plan"), None);
    }

    #[test]
    fn autolink_form_rewritten() {
        let src = "<attachments/plan/a.pdf>\n";
        let out = rewrite_links(src, "attachments/plan", "attachments/p/plan").unwrap();
        assert_eq!(out, "<attachments/p/plan/a.pdf>\n");
    }

    #[test]
    fn no_touch_returns_none() {
        assert_eq!(rewrite_links("plain text attachments/plan/a.png\n", "attachments/plan", "attachments/p/plan"), None);
    }
}
