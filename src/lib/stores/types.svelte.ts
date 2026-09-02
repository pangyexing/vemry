// The vault's block types (`.vemry/types/*.md`), mirrored in process for the editor's
// `/` menu and the block menu (a keystroke cannot await an IPC). Refreshed when a vault
// opens and on the watcher's types_changed event.
import { api, type BlockType } from "$lib/api";

class TypesStore {
  list = $state<BlockType[]>([]);

  /** The enabled types, sorted by name (what the menus show). */
  get enabled(): BlockType[] {
    return this.list.filter((t) => !t.disabled);
  }

  async refresh() {
    try {
      this.list = await api.typesList();
    } catch {
      this.list = [];
    }
  }
}

export const types = new TypesStore();
