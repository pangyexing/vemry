// The feedback dialog (FeedbackDialog.svelte): whether it is open, and the draft. The draft
// outlives a close without sending — a second thought does not cost the text — and is
// cleared once the server has taken it. Nothing here is stored: a restart forgets it.
import type { FeedbackKind } from "$lib/api";

/** The longest text the server takes, in characters (feedback.rs on both sides). */
export const FEEDBACK_MAX = 5000;

class FeedbackStore {
  open = $state(false);
  kind = $state<FeedbackKind>("bug");
  text = $state("");
  /** The reply address typed while signed out; signed in, the account's is used instead. */
  email = $state("");
  diagnostics = $state(true);

  show() {
    this.open = true;
  }

  close() {
    this.open = false;
  }

  /** Sent: the draft goes with it (the kind and the ticked box stay as the user left them). */
  sent() {
    this.text = "";
    this.open = false;
  }
}

export const feedback = new FeedbackStore();
