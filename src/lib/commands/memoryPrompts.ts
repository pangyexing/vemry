// The preset prompt of "initialise VEMRY.md": rather than dropping a static template,
// it has the agent survey the vault and draft or review it — the writing goes through
// write_note/edit_note, the diff is confirmed by the user through the PermissionGate,
// and a new user meets the approval flow for the first time along the way.
// A nearly empty vault (with nothing to analyse) still falls back to
// init_memory_file's static template.

import { t } from "$lib/i18n/i18n.svelte";

/** Both prompts are addressed to the model rather than to the interface — but since
 *  the model "answers in the user's language", they follow the interface language too:
 *  only then is the VEMRY.md drafted under an English interface itself English. */
export const MEMORY_DRAFT_PROMPT = () => t("memoryPrompt.draft");

export const MEMORY_REVIEW_PROMPT = () => t("memoryPrompt.review");
