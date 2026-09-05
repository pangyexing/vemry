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
pub const TIDY_NOTE: &str = include_str!("../../examples/demo-vault/.vemry/skills/tidy-note.md");
pub const PRD: &str = include_str!("../../examples/demo-vault/.vemry/skills/prd.md");
pub const DECISION_LOG: &str = include_str!("../../examples/demo-vault/.vemry/skills/decision-log.md");
pub const FEEDBACK_TRIAGE: &str = include_str!("../../examples/demo-vault/.vemry/skills/feedback-triage.md");
pub const ITERATION_PLAN: &str = include_str!("../../examples/demo-vault/.vemry/skills/iteration-plan.md");
pub const RETRO: &str = include_str!("../../examples/demo-vault/.vemry/skills/retro.md");
pub const BRIEF: &str = include_str!("../../examples/demo-vault/.vemry/skills/brief.md");
pub const TUTOR: &str = include_str!("../../examples/demo-vault/.vemry/skills/tutor.md");

/// One installable example block type (`.vemry/types/<file>`): the same shape as a skill example.
pub struct ExampleType {
    pub file: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub body: &'static str,
}

/// The example block types Settings offers: the product-document blocks and the flashcard.
pub const TYPE_EXAMPLES: &[ExampleType] = &[
    ExampleType { file: "requirement.md", name: "requirement", description: "One product requirement, with its priority, status and owner on the line", body: include_str!("../../examples/demo-vault/.vemry/types/requirement.md") },
    ExampleType { file: "decision.md", name: "decision", description: "A product decision — indexed as a claim, so evidence supports it and the Claims view shows which decisions have none", body: include_str!("../../examples/demo-vault/.vemry/types/decision.md") },
    ExampleType { file: "question.md", name: "question", description: "An open question in a spec or a research note, with who owns the answer", body: include_str!("../../examples/demo-vault/.vemry/types/question.md") },
    ExampleType { file: "hypothesis.md", name: "hypothesis", description: "A hypothesis behind a change — the metric, the expected result, and a review date; the review shows up in the Agenda when it is due", body: include_str!("../../examples/demo-vault/.vemry/types/hypothesis.md") },
    ExampleType { file: "card.md", name: "card", description: "A flashcard — the first line is the question, the body is the answer; the Review view shows it when it is due and writes the grade back onto the line", body: include_str!("../../examples/demo-vault/.vemry/types/card.md") },
];

pub fn example_type(name: &str) -> Option<&'static ExampleType> {
    TYPE_EXAMPLES.iter().find(|e| e.name == name)
}

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
    ExampleSkill {
        file: "tidy-note.md",
        name: "tidy-note",
        description: "Restructuring one messy note: read it whole, propose an outline, move blocks without rewording them, keep every id and every private block",
        body: TIDY_NOTE,
    },
    ExampleSkill { file: "prd.md", name: "prd", description: "From scattered input to a PRD: background, goal, requirement blocks with their sources, decisions, open questions, a change log", body: PRD },
    ExampleSkill { file: "decision-log.md", name: "decision-log", description: "Decisions and their evidence: which decisions have none, which feedback contradicts one, what a decision superseded", body: DECISION_LOG },
    ExampleSkill { file: "feedback-triage.md", name: "feedback-triage", description: "User feedback into blocks with feature, sentiment and source, linked to the requirement or decision it bears on", body: FEEDBACK_TRIAGE },
    ExampleSkill { file: "iteration-plan.md", name: "iteration-plan", description: "Planning the next iteration: score the backlog, pick within capacity, tag the chosen blocks with the iteration, write the iteration note", body: ITERATION_PLAN },
    ExampleSkill { file: "retro.md", name: "retro", description: "Closing an iteration: what shipped, which hypotheses held, which decisions changed, what carries over — and the release notes", body: RETRO },
    ExampleSkill { file: "brief.md", name: "brief", description: "Exporting an implementation brief for a coding agent, and filling the record back in when the work returns", body: BRIEF },
    ExampleSkill { file: "tutor.md", name: "tutor", description: "The learning method — explain from what the note already says, make flashcards as [!card] blocks, and quiz from the cards that are due", body: TUTOR },
];

pub fn example(name: &str) -> Option<&'static ExampleSkill> {
    EXAMPLES.iter().find(|e| e.name == name)
}
