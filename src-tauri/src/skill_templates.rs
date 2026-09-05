//! The example skills a vault can install from Settings (and the research skill the
//! panel installs by itself): instruction-level playbooks for the routines the agent
//! is meant to run — the daily start and close, meeting notes into action items, the
//! weekly report. Each is a `.vemry/skills/<file>.md` with the frontmatter the skill
//! loader reads (`name` / `description`); the body is what `read_skill` hands the
//! model. Installing never overwrites: a file already there is the user's.
//!
//! The templates are English: a skill is read by the model, and the system prompt
//! already tells it to answer in the user's language (design constraint 3 in
//! CLAUDE.md). Copies of the same files live in examples/demo-vault/.vemry/skills.

/// One installable example: the file name under `.vemry/skills/`, the `name`
/// the frontmatter carries, and the whole file.
pub struct ExampleSkill {
    pub file: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

pub const RESEARCH: &str = include_str!("../../examples/demo-vault/.vemry/skills/research.md");
pub const DAILY_START: &str = include_str!("../../examples/demo-vault/.vemry/skills/daily-start.md");
pub const DAILY_CLOSE: &str = include_str!("../../examples/demo-vault/.vemry/skills/daily-close.md");
pub const MEETING_ACTIONS: &str = include_str!("../../examples/demo-vault/.vemry/skills/meeting-actions.md");
pub const WEEKLY_REPORT: &str = include_str!("../../examples/demo-vault/.vemry/skills/weekly-report.md");
pub const TIDY: &str = include_str!("../../examples/demo-vault/.vemry/skills/tidy.md");

/// The catalogue Settings offers. `description` is shown before installing (the
/// same sentence as the file's frontmatter, so the list and the installed skill
/// say the same thing).
pub const EXAMPLES: &[ExampleSkill] = &[
    ExampleSkill {
        file: "research.md",
        name: "research",
        description: "The research method — plan first, the vault before the web, verify and cite every source, and let the conclusion settle into a note",
        body: RESEARCH,
    },
    ExampleSkill {
        file: "daily-start.md",
        name: "daily-start",
        description: "The morning routine: what is overdue, due today or in progress, what yesterday's journal promised — into three things for today",
        body: DAILY_START,
    },
    ExampleSkill {
        file: "daily-close.md",
        name: "daily-close",
        description: "The evening routine: what got done and what changed today, appended to today's journal note",
        body: DAILY_CLOSE,
    },
    ExampleSkill {
        file: "meeting-actions.md",
        name: "meeting-actions",
        description: "Meeting notes into action items: who, what, by when — each quoting its sentence, deduplicated, reviewed on the card, then written into the project notes",
        body: MEETING_ACTIONS,
    },
    ExampleSkill {
        file: "weekly-report.md",
        name: "weekly-report",
        description: "The weekly report: what shipped this week, what is blocked, what is planned next — appended to the week's note, with a copyable version for others",
        body: WEEKLY_REPORT,
    },
    ExampleSkill {
        file: "tidy.md",
        name: "tidy",
        description: "Tidying a folder or the vault: audit first, propose every move and fix as a list the user reviews, then move with links rewritten — nothing is lost",
        body: TIDY,
    },
];

pub fn example(name: &str) -> Option<&'static ExampleSkill> {
    EXAMPLES.iter().find(|e| e.name == name)
}
