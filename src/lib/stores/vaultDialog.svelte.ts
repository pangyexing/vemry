// The one question asked about a folder: the name it is shown by. Asked before a folder
// that is not a vault yet is first opened (nothing in it is read until the answer), when
// a vault is created, and on a rename. The caller awaits the answer; null = cancelled.

export type VaultDialogMode = "open" | "create" | "rename";

interface VaultDialogRequest {
  mode: VaultDialogMode;
  /** The folder the vault is (or will be). */
  folder: string;
  /** The name the field starts with. */
  name: string;
  resolve: (name: string | null) => void;
}

class VaultDialogStore {
  pending = $state<VaultDialogRequest | null>(null);

  /** Ask for a name; a question still open is answered as cancelled first. */
  ask(mode: VaultDialogMode, folder: string, name: string): Promise<string | null> {
    this.pending?.resolve(null);
    return new Promise((resolve) => {
      this.pending = { mode, folder, name, resolve };
    });
  }

  answer(name: string | null) {
    const req = this.pending;
    this.pending = null;
    req?.resolve(name);
  }
}

export const vaultDialog = new VaultDialogStore();

/** Whether the manage-vaults dialog is open (VaultManager.svelte). */
export const vaultManager = $state({ open: false });
