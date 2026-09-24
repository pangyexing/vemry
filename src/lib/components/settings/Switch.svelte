<script lang="ts">
  import { getContext } from "svelte";
  import type { RowContext } from "./context";

  // An on/off setting: a button with the switch role, so the row's <label for> reaches it
  // (a button is labelable) and a click on the label flips it like a click on the knob.
  let {
    checked,
    onchange,
    disabled = false,
    label,
  }: {
    checked: boolean;
    onchange: (v: boolean) => void;
    disabled?: boolean;
    /** Outside a setting row (a skill or a script in a list): the name the switch is for. */
    label?: string;
  } = $props();
  const row = getContext<RowContext | undefined>("setting-row");
</script>

<button
  id={label ? undefined : row?.ctlId}
  class="switch"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  aria-labelledby={label ? undefined : row ? `${row.ctlId}-label` : undefined}
  {disabled}
  onclick={() => onchange(!checked)}
><span class="knob"></span></button>

<style>
  .switch {
    position: relative;
    flex: none;
    width: 30px;
    height: 18px;
    padding: 0;
    border-radius: 9px;
    background: var(--border-strong);
    transition: background 0.15s;
  }
  .switch.on {
    background: var(--accent);
  }
  .switch:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    /* --bg rather than white: on the dark theme's light accent a white knob would glare */
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: left 0.15s;
  }
  .switch.on .knob {
    left: 14px;
  }
  .switch:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .switch,
    .knob {
      transition: none;
    }
  }
</style>
