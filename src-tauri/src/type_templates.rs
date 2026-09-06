//! The example block types a vault can install from Settings: `.vemry/types/<file>`
//! files with the frontmatter the type loader reads. Installing never overwrites: a
//! file already there is the user's. (Skills are not installed this way any more —
//! the built-in ones ship inside vemry-agent and a vault customises one by taking a
//! copy; see `vemry_agent::skills`.)
//!
//! The templates are English: a type's description is read by the model, and the
//! system prompt already tells it to answer in the user's language (design
//! constraint 3 in CLAUDE.md). The files themselves live in
//! examples/demo-vault/.vemry/types, so the demo vault and the catalogue cannot drift.

/// One installable example block type: the file name under `.vemry/types/`, the
/// `name` the frontmatter carries, and the whole file.
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
