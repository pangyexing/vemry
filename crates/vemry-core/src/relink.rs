//! Link rewriting when a note or a file moves: the references that pointed at the
//! old place are rewritten to the new one, in the spelling the author used (a bare
//! name stays a bare name while it still resolves uniquely, a path stays a path,
//! an encoded Markdown target stays encoded), everything else byte for byte, code
//! fences untouched. Pure planning: nothing here writes — the caller (the agent's
//! move tools, behind the permission gate) shows the plan as a diff and lands it.

use crate::attach_migrate::encode_target;
use crate::parser::{is_fence_close, parse_fence_open, parse_note};
use crate::{NoteId, Vault};

/// One note whose text changes because something it linked to moved.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Relink {
    pub path: String,
    /// How many link occurrences change in this note.
    pub links: usize,
    #[serde(skip)]
    pub old: String,
    #[serde(skip)]
    pub new: String,
}

impl Vault {
    /// The notes whose wikilinks must change when `from` becomes `to`: every
    /// `[[…]]` that resolves to `from` (its heading, block ref and alias kept) is
    /// respelled — the new stem when a bare name would still resolve to the moved
    /// note uniquely, otherwise the new path. The moved note itself is included
    /// (a link to its own old name). Notes the index does not hold are not read.
    pub fn plan_note_move_links(&self, from: &NoteId, to: &NoteId) -> Vec<Relink> {
        let to_path = to.as_str().strip_suffix(".md").unwrap_or(to.as_str()).to_string();
        let to_stem = to.stem().to_string();
        // A bare name resolves through the name table: it is unambiguous only when no
        // other note (the moved one excluded) already answers to the new stem
        let bare_ok = !self
            .index()
            .summaries()
            .iter()
            .any(|s| &s.id != from && &s.id != to && s.id.stem().eq_ignore_ascii_case(&to_stem));
        let mut sources: Vec<NoteId> = self.index().backlinks_of(from);
        if !sources.contains(from) {
            sources.push(from.clone());
        }
        let mut out = Vec::new();
        for src in sources {
            let Ok(raw) = self.read_note(&src) else { continue };
            let parsed = parse_note(&raw);
            let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
            for b in &parsed.blocks {
                if matches!(b.kind, crate::BlockKind::Private) {
                    continue;
                }
                for l in &b.links {
                    if l.target.is_empty() {
                        continue;
                    }
                    let resolved = self.index().resolve_target(&l.target, &src);
                    if resolved.as_ref() != Some(from) {
                        continue;
                    }
                    let spelled_as_path = l.target.contains('/');
                    let target = if spelled_as_path || !bare_ok { to_path.clone() } else { to_stem.clone() };
                    let mut inner = target;
                    if let Some(h) = &l.heading {
                        inner.push('#');
                        inner.push_str(h);
                    } else if let Some(id) = &l.block_ref {
                        inner.push_str("#^");
                        inner.push_str(id);
                    }
                    if let Some(a) = &l.alias {
                        inner.push('|');
                        inner.push_str(a);
                    }
                    edits.push((l.span.clone(), format!("[[{inner}]]")));
                }
            }
            if edits.is_empty() {
                continue;
            }
            edits.sort_by_key(|(r, _)| r.start);
            let mut new = String::with_capacity(raw.len());
            let mut pos = 0;
            for (r, rep) in &edits {
                if r.start < pos {
                    continue; // overlapping spans never happen; defensive
                }
                new.push_str(&raw[pos..r.start]);
                new.push_str(rep);
                pos = r.end;
            }
            new.push_str(&raw[pos..]);
            if new != raw {
                // The moved note's own text is keyed by its new path: the caller writes it after the rename
                let path = if &src == from { to.to_string() } else { src.to_string() };
                out.push(Relink { path, links: edits.len(), old: raw, new });
            }
        }
        out
    }

    /// The notes whose references to a non-note file must change when it moves from
    /// `from_rel` to `to_rel`: Markdown links and embeds (`](path)`, raw and
    /// encoded), angle-bracket autolinks and wikilink embeds (`[[path`), plus the
    /// bare-filename form (`![[img.png]]`) when the file name changes and the old
    /// name was unique among the vault's files. Code fences are skipped.
    pub fn plan_file_move_links(&self, from_rel: &str, to_rel: &str) -> Vec<Relink> {
        let from_name = from_rel.rsplit('/').next().unwrap_or(from_rel);
        let to_name = to_rel.rsplit('/').next().unwrap_or(to_rel);
        let mut pairs: Vec<(String, String)> = vec![
            (format!("]({from_rel})"), format!("]({to_rel})")),
            (format!("]({})", encode_target(from_rel)), format!("]({})", encode_target(to_rel))),
            (format!("<{from_rel}>"), format!("<{to_rel}>")),
            (format!("[[{from_rel}]]"), format!("[[{to_rel}]]")),
            (format!("[[{from_rel}|"), format!("[[{to_rel}|")),
        ];
        if from_name != to_name {
            // A bare file name resolves by shortest path: rewrite it only when it named this file alone
            let same_name = self.list_files().iter().filter(|f| f.rsplit('/').next() == Some(from_name)).count();
            if same_name <= 1 {
                pairs.push((format!("[[{from_name}]]"), format!("[[{to_name}]]")));
                pairs.push((format!("[[{from_name}|"), format!("[[{to_name}|")));
                pairs.push((format!("]({from_name})"), format!("]({to_name})")));
                pairs.push((format!("]({})", encode_target(from_name)), format!("]({})", encode_target(to_name))));
            }
        }
        let mut out = Vec::new();
        for s in self.index().summaries() {
            let Ok(raw) = self.read_note(&s.id) else { continue };
            let (new, count) = replace_outside_fences(&raw, &pairs);
            if count > 0 {
                out.push(Relink { path: s.id.to_string(), links: count, old: raw, new });
            }
        }
        out
    }
}

/// Literal substitutions applied to every line outside code fences; returns the
/// new text and how many replacements were made.
fn replace_outside_fences(content: &str, pairs: &[(String, String)]) -> (String, usize) {
    let mut out = String::with_capacity(content.len());
    let mut count = 0;
    let mut fence: Option<(char, usize)> = None;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if let Some((ch, len)) = fence {
            if is_fence_close(trimmed, ch, len) {
                fence = None;
            }
            out.push_str(line);
            continue;
        }
        if let Some((ch, len, _)) = parse_fence_open(trimmed) {
            fence = Some((ch, len));
            out.push_str(line);
            continue;
        }
        let mut cur = line.to_string();
        for (old, new) in pairs {
            let n = cur.matches(old.as_str()).count();
            if n > 0 {
                cur = cur.replace(old.as_str(), new.as_str());
                count += n;
            }
        }
        out.push_str(&cur);
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let abs = dir.path().join(p);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, c).unwrap();
        }
        let v = Vault::open(dir.path()).unwrap();
        (dir, v)
    }

    #[test]
    fn note_move_respells_links_in_the_authors_form() {
        let (_d, v) = vault_with(&[
            ("inbox/Idea.md", "# Idea\n\nsee [[Idea#Top|me]] ^k1\n\n## Top\n"),
            ("A.md", "bare [[Idea]] and path [[inbox/Idea#^k1]] and ![[Idea]] and alias [[inbox/Idea|the idea]]\n```\n[[Idea]] in code\n```\n"),
            ("B.md", "unrelated [[Other]]\n"),
        ]);
        let from = NoteId::new("inbox/Idea.md");
        let to = NoteId::new("projects/Idea Two.md");
        let plan = v.plan_note_move_links(&from, &to);
        let a = plan.iter().find(|r| r.path == "A.md").unwrap();
        assert_eq!(a.links, 4);
        assert_eq!(a.new, "bare [[Idea Two]] and path [[projects/Idea Two#^k1]] and ![[Idea Two]] and alias [[projects/Idea Two|the idea]]\n```\n[[Idea]] in code\n```\n");
        let own = plan.iter().find(|r| r.path == "projects/Idea Two.md").unwrap();
        assert_eq!(own.new, "# Idea\n\nsee [[Idea Two#Top|me]] ^k1\n\n## Top\n");
        assert!(!plan.iter().any(|r| r.path == "B.md"));
    }

    #[test]
    fn note_move_uses_the_path_when_the_new_name_is_taken() {
        let (_d, v) = vault_with(&[("Idea.md", "# Idea\n"), ("archive/Plan.md", "# Plan\n"), ("A.md", "[[Idea]]\n")]);
        let plan = v.plan_note_move_links(&NoteId::new("Idea.md"), &NoteId::new("new/Plan.md"));
        assert_eq!(plan[0].new, "[[new/Plan]]\n");
    }

    #[test]
    fn file_move_rewrites_embeds_and_links_outside_fences() {
        let (_d, v) = vault_with(&[
            ("A.md", "![[shot.png]] and ![](attachments/A/shot.png) and [x](attachments/A/shot%20two.png) <attachments/A/shot.png>\n```\n![[shot.png]]\n```\n"),
        ]);
        std::fs::create_dir_all(v.root().join("attachments/A")).unwrap();
        std::fs::write(v.root().join("attachments/A/shot.png"), b"x").unwrap();
        let plan = v.plan_file_move_links("attachments/A/shot.png", "assets/gw/shot-1.png");
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].links, 3);
        assert_eq!(plan[0].new, "![[shot-1.png]] and ![](assets/gw/shot-1.png) and [x](attachments/A/shot%20two.png) <assets/gw/shot-1.png>\n```\n![[shot.png]]\n```\n");
    }
}
