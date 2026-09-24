<script lang="ts" module>
  import type { SettingsUpdate, SettingsView } from "$lib/api";
  /** The settings that are one on/off field. */
  export type SwitchField = {
    [K in keyof SettingsView & keyof SettingsUpdate]: SettingsView[K] extends boolean ? K : never;
  }[keyof SettingsView & keyof SettingsUpdate];
</script>

<script lang="ts">
  import { form } from "./form.svelte";
  import SettingRow from "./SettingRow.svelte";
  import Switch from "./Switch.svelte";

  // A row that is one on/off field: the row's words from the index, the switch written through at once.
  let { id, field, hint }: { id: string; field: SwitchField; hint?: string } = $props();
</script>

{#if form.s}
  <SettingRow {id} {hint}>
    <Switch checked={!!form.s[field]} onchange={(v) => void form.set({ [field]: v }, id)} />
  </SettingRow>
{/if}
