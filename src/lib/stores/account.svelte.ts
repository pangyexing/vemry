// The in-process mirror of the hosted account (Rust's account.rs is the single source
// of truth; account.json on disk is its file). Every surface that shows the account —
// the settings page's account tab, the provider toggle's model list, the status bar's
// quota chip — reads this one object, and it is kept in step by the account event
// Rust emits on every change (a sign-in, a refresh after a hosted turn, a token
// revoked and noticed by any request). App loads it once at startup.
import { api, onAccountChanged, type AccountStatus } from "$lib/api";

class AccountStore {
  status = $state<AccountStatus | null>(null);

  get loggedIn(): boolean {
    return this.status?.logged_in ?? false;
  }
  get me() {
    return this.status?.me ?? null;
  }
  /** Hosted requests may go out right now (signed in, hosted access, the snapshot not expired). */
  get hostedOk(): boolean {
    return this.status?.hosted_ok ?? false;
  }
  /** The hosted model catalog (empty while signed out). */
  get models() {
    return this.status?.me?.models ?? [];
  }

  /** Read the status once (startup) and follow every change from then on. */
  async init() {
    try {
      this.status = await api.accountStatus();
    } catch {
      // No IPC yet (or an old backend): stays signed out until an event arrives
    }
    void onAccountChanged((s) => (this.status = s));
  }

  /** The snapshot fetched again; a failure keeps the current one (Rust reports it). */
  async refresh(): Promise<void> {
    this.status = await api.accountRefresh();
  }
}

export const account = new AccountStore();
