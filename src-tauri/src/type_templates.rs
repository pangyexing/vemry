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

use std::sync::OnceLock;

/// The example files, in the order Settings offers them: the product-document blocks
/// and the flashcard.
const TYPE_FILES: &[(&str, &str)] = &[
    ("requirement.md", include_str!("../../examples/demo-vault/.vemry/types/requirement.md")),
    ("decision.md", include_str!("../../examples/demo-vault/.vemry/types/decision.md")),
    ("question.md", include_str!("../../examples/demo-vault/.vemry/types/question.md")),
    ("hypothesis.md", include_str!("../../examples/demo-vault/.vemry/types/hypothesis.md")),
    ("card.md", include_str!("../../examples/demo-vault/.vemry/types/card.md")),
];

/// One installable example block type: the file name under `.vemry/types/`, the whole
/// file, and the `name` and `description` read out of its own frontmatter by the type
/// loader's own parser — so Settings can never describe a type differently from what
/// the model is given.
pub struct ExampleType {
    pub file: &'static str,
    pub name: String,
    pub description: String,
    pub body: &'static str,
}

/// The example block types Settings offers, parsed once.
pub fn type_examples() -> &'static [ExampleType] {
    static ALL: OnceLock<Vec<ExampleType>> = OnceLock::new();
    ALL.get_or_init(|| {
        TYPE_FILES
            .iter()
            .map(|(file, body)| {
                let t = vemry_core::parse_block_type(file, body);
                ExampleType { file, name: t.name, description: t.description, body }
            })
            .collect()
    })
}

pub fn example_type(name: &str) -> Option<&'static ExampleType> {
    type_examples().iter().find(|e| e.name == name)
}
