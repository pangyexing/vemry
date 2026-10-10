//! The vault's link health, read off the index: unresolved wikilinks, orphan notes
//! (nothing links to them), evidence whose `supports::` points at no claim, and
//! clips of the same page. Everything but the anchor checks is read off the index;
//! `[[note#heading]]` / `[[note#^id]]` go through `resolve_ref`, which reads the
//! target note. The agent's `audit_links` tool reads `audit_links_for_agent`, where a
//! private note does not exist, and filters the rows whose source is private out of
//! the report before the model sees it.

use std::collections::HashMap;

use crate::{BlockKind, NoteId, RefResolution, RefTarget, Vault};

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct AuditReport {
    /// (the linking note, the block handle, the target as written)
    pub unresolved: Vec<(NoteId, String, String)>,
    /// Notes nothing links to (the vault's own files excluded).
    pub orphans: Vec<NoteId>,
    /// (the evidence note, the block handle, the `supports::` target that reaches no claim)
    pub dangling_supports: Vec<(NoteId, String, String)>,
    /// Clips of one normalised source URL: (url, the notes holding it), only groups of two or more.
    pub duplicate_clips: Vec<(String, Vec<NoteId>)>,
    /// (the linking note, the block handle, the target) — a `[[note#heading]]` or
    /// `[[note#^id]]` whose note exists but whose anchor does not (a renamed heading,
    /// a block that moved without its links).
    pub dangling_anchors: Vec<(NoteId, String, String)>,
}

impl Vault {
    /// The link health of the vault, or of one folder (the notes under it as sources;
    /// orphans are judged against the whole vault's backlinks).
    pub fn audit_links(&self, folder: Option<&str>) -> AuditReport {
        self.audit(folder, false)
    }

    /// `audit_links` as the agent may see it: a private note does not exist. A link
    /// to one is unresolved (what get_links calls it), a backlink from one does not
    /// save a note from being an orphan, and an anchor or a `supports::` is resolved as
    /// the agent resolves it (`resolve_ref_for_agent`, a private note counting as
    /// missing) — so no section can tell the model that a private note is there. The
    /// rows whose own source is private stay in, for the tool to filter and count like
    /// every other read.
    pub fn audit_links_for_agent(&self, folder: Option<&str>) -> AuditReport {
        self.audit(folder, true)
    }

    fn audit(&self, folder: Option<&str>, hide_private: bool) -> AuditReport {
        let hidden = |id: &NoteId| hide_private && self.is_private(id);
        let resolve = |target: &str, from: &NoteId| {
            if hide_private {
                self.resolve_ref_for_agent(target, Some(from))
            } else {
                self.resolve_ref(target, Some(from))
            }
        };
        let prefix = folder.map(|f| f.trim_matches('/')).filter(|f| !f.is_empty()).map(|f| format!("{f}/"));
        let in_scope = |id: &NoteId| match &prefix {
            Some(p) => id.as_str().starts_with(p.as_str()),
            None => true,
        };
        let mut report = AuditReport::default();
        let mut by_source: HashMap<String, Vec<NoteId>> = HashMap::new();
        let mut ids: Vec<NoteId> = self.index().ids().cloned().collect();
        ids.sort();
        for id in &ids {
            if !in_scope(id) {
                continue;
            }
            for edge in self.index().edges(id) {
                let gone = edge.resolved.as_ref().is_none_or(hidden);
                if gone && !edge.raw_target.is_empty() {
                    report.unresolved.push((id.clone(), edge.from_block.clone(), edge.raw_target.clone()));
                }
            }
            let linked = if hide_private {
                self.index().backlinks_of(id).iter().any(|src| !self.is_private(src))
            } else {
                self.index().has_backlinks(id)
            };
            if !linked && id.as_str() != "VEMRY.md" {
                report.orphans.push(id.clone());
            }
            if let Some(blocks) = self.index().blocks(id) {
                for b in blocks {
                    for l in &b.links {
                        if l.heading.is_none() && l.block_ref.is_none() {
                            continue;
                        }
                        let target = l.ref_target();
                        // An anchor into a private note: that note is not there, which the
                        // unresolved section already says
                        if matches!(resolve(&target, id), Ok(RefResolution::AnchorMissing { note, .. }) if !hidden(&note)) {
                            report.dangling_anchors.push((id.clone(), b.handle(), target));
                        }
                    }
                    let BlockKind::Evidence { supports } = &b.kind else { continue };
                    for target in supports {
                        let ok = matches!(
                            resolve(target, id),
                            Ok(RefResolution::Resolved(hit))
                                if matches!(hit.target, RefTarget::Block { block_kind: BlockKind::Claim, .. }) && !hidden(&hit.note)
                        );
                        if !ok {
                            report.dangling_supports.push((id.clone(), b.handle(), target.clone()));
                        }
                    }
                }
            }
            if id.as_str().starts_with(crate::weburl::CLIPS_DIR) {
                if let Some(src) = self.index().summary(id).and_then(|s| s.source) {
                    if let Ok(u) = crate::weburl::normalize_url(&src) {
                        by_source.entry(u.to_string()).or_default().push(id.clone());
                    }
                }
            }
        }
        let mut dups: Vec<(String, Vec<NoteId>)> = by_source.into_iter().filter(|(_, v)| v.len() > 1).collect();
        dups.sort();
        report.duplicate_clips = dups;
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_finds_unresolved_orphans_dangling_and_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, c: &str| {
            let abs = dir.path().join(p);
            std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
            std::fs::write(abs, c).unwrap();
        };
        w("A.md", "links [[B]] and [[Missing]]\n\n> [!claim] sleep matters ^c1\n");
        w("B.md", "# B\n\n> [!evidence] walker 2017 supports:[[A#^c1]] supports:[[A#^nope]]\n");
        w("Lonely.md", "nobody links here, see [[A#Gone heading]] and [[A#^c1]]\n");
        w("clips/One.md", "---\nsource: https://Example.com/p\n---\n# One\n");
        w("clips/Two.md", "---\nsource: https://example.com/p\n---\n# Two\n");
        let v = Vault::open(dir.path()).unwrap();
        let r = v.audit_links(None);
        assert_eq!(r.unresolved.len(), 1);
        assert_eq!(r.unresolved[0].2, "Missing");
        let orphans: Vec<&str> = r.orphans.iter().map(|n| n.as_str()).collect();
        // A is linked from B's supports:: (a link like any other), so only Lonely and the clips are orphans
        assert!(orphans.contains(&"Lonely.md") && !orphans.contains(&"A.md") && !orphans.contains(&"B.md"), "{orphans:?}");
        assert_eq!(r.dangling_supports.len(), 1);
        assert_eq!(r.dangling_supports[0].2, "A#^nope");
        assert_eq!(r.duplicate_clips.len(), 1);
        assert_eq!(r.duplicate_clips[0].1.len(), 2);
        // a dangling supports:: is a dangling anchor too (it is a wikilink to a block); the heading one joins it
        assert_eq!(r.dangling_anchors.len(), 2, "{:?}", r.dangling_anchors);
        assert!(r.dangling_anchors.iter().any(|(_, _, t)| t == "A#Gone heading"));
        let scoped = v.audit_links(Some("clips"));
        assert!(scoped.unresolved.is_empty() && scoped.duplicate_clips.len() == 1);
    }

    /// The agent's report cannot tell a private note is there: a link to one is
    /// unresolved, a backlink from one does not count, and an anchor or a support into
    /// one is judged missing rather than "the note exists, the anchor does not". The
    /// full report, for the user, still sees everything.
    #[test]
    fn the_agents_audit_does_not_reveal_private_notes() {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, c: &str| std::fs::write(dir.path().join(p), c).unwrap();
        w("Secret.md", "---\nprivate: true\n---\n> [!claim] hidden claim ^c1\n\nlinks to [[Lonely]]\n");
        w("Lonely.md", "only a private note links here\n");
        w("Public.md", "see [[Secret]] and [[Secret#^gone]]\n\n> [!evidence] e supports:[[Secret#^c1]]\n");
        let v = Vault::open(dir.path()).unwrap();
        let full = v.audit_links(None);
        let agent = v.audit_links_for_agent(None);
        let names = |rows: &[(NoteId, String, String)]| rows.iter().map(|r| r.2.clone()).collect::<Vec<_>>();
        assert!(!full.orphans.contains(&NoteId::new("Lonely.md")), "the user sees the private backlink");
        assert!(agent.orphans.contains(&NoteId::new("Lonely.md")), "the agent sees an orphan");
        assert!(!names(&full.unresolved).contains(&"Secret".to_string()));
        assert!(names(&agent.unresolved).contains(&"Secret".to_string()), "a link to a private note is unresolved");
        assert!(names(&full.dangling_anchors).iter().any(|t| t.contains("gone")));
        assert!(agent.dangling_anchors.is_empty(), "no 'the note exists' for a private note");
        assert!(full.dangling_supports.is_empty());
        assert_eq!(agent.dangling_supports.len(), 1, "a support into a private claim reaches no claim");
    }

}
