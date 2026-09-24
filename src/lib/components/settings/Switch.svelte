<script lang="ts">
  import { rowControl } from "./context";

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
  /** Inside a row the label is the row's (its id comes from rowControl); in a list, the name given. */
  const labelled = (el: HTMLElement) => {
    if (label) return;
    rowControl(el);
    el.setAttribute("aria-labelledby", `${el.id}-label`);
  };
</script>

<button
  {@attach labelled}
  class="switch"
  class:on={checked}
  role="switch"
  aria-checked={checked}
  aria-label={label}
  {disabled}
  onclick={() => onchange(!checked)}
><span class="knob"></span></button>

<style>
  /* The same geometry as the lab block's switch (app.css .cm-lab-switch): one switch in the app */
  .switch {
    position: relative;
    flex: none;
    width: 30px;
    height: 17px;
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
    width: 13px;
    height: 13px;
    border-radius: 50%;
    /* --bg rather than white: on the dark theme's light accent a white knob would glare */
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: left 0.15s;
  }
  .switch.on .knob {
    left: 15px;
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
