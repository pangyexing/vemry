//! The vault's link health, read off the index: unresolved wikilinks, orphan notes
//! (nothing links to them), evidence whose `supports::` points at no claim, and
//! clips of the same page. Everything but the anchor checks is read off the index;
//! `[[note#heading]]` / `[[note#^id]]` go through `resolve_ref`, which reads the
//! target note. The agent's `audit_links` tool filters private notes out of the
//! report before the model sees it.

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
                if edge.resolved.is_none() && !edge.raw_target.is_empty() {
                    report.unresolved.push((id.clone(), edge.from_block.clone(), edge.raw_target.clone()));
                }
            }
            if !self.index().has_backlinks(id) && id.as_str() != "VEMRY.md" {
                report.orphans.push(id.clone());
            }
            if let Some(blocks) = self.index().blocks(id) {
                for b in blocks {
                    for l in &b.links {
                        if l.heading.is_none() && l.block_ref.is_none() {
                            continue;
                        }
                        let target = l.ref_target();
                        if matches!(self.resolve_ref(&target, Some(id)), Ok(RefResolution::AnchorMissing { .. })) {
                            report.dangling_anchors.push((id.clone(), b.handle(), target));
                        }
                    }
                    let BlockKind::Evidence { supports } = &b.kind else { continue };
                    for target in supports {
                        let ok = matches!(
                            self.resolve_ref(target, Some(id)),
                            Ok(RefResolution::Resolved(hit)) if matches!(hit.target, RefTarget::Block { block_kind: BlockKind::Claim, .. })
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
}
