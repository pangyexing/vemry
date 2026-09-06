<script lang="ts" module>
  /** The one definition of the seven icon names: AgentChat's Chip.kind and AtPicker's
   *  iconKind both take their type from here, and an eighth changes this one place
   *  (and the template below). */
  export type RefIconKind = "note" | "folder" | "file" | "image" | "page" | "reading" | "selection" | "block" | "view" | "skill";
</script>

<script lang="ts">
  // The seven line icons of a context reference (the same hand-drawn style as the file
  // tree and the activity bar: a 20 viewport with a 1.6 stroke), shared by the @
  // candidate rows, the composer chips and a user reference in the timeline — one
  // drawing for three places, instead of each reaching for an emoji (which every
  // platform's fonts render differently).
  let {
    kind,
    size = 14,
  }: {
    kind: RefIconKind;
    size?: number;
  } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 20 20"
  fill="none"
  stroke="currentColor"
  stroke-width="1.6"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#if kind === "note"}
    <path d="M5.5 2.5 H12 L15.5 6 V16.5 A1 1 0 0 1 14.5 17.5 H5.5 A1 1 0 0 1 4.5 16.5 V3.5 A1 1 0 0 1 5.5 2.5 Z" />
    <path d="M12 2.5 V6 H15.5 M7.5 10 H12.5 M7.5 13.25 H11" />
  {:else if kind === "folder"}
    <path d="M2 5.25 A1.75 1.75 0 0 1 3.75 3.5 H7.1 L9 5.6 H16.25 A1.75 1.75 0 0 1 18 7.35 V14.75 A1.75 1.75 0 0 1 16.25 16.5 H3.75 A1.75 1.75 0 0 1 2 14.75 Z" />
  {:else if kind === "file"}
    <path d="M15.2 9.4 L9.6 15 A3.6 3.6 0 0 1 4.5 9.9 L10.5 3.9 A2.4 2.4 0 0 1 13.9 7.3 L8.1 13.1 A1.2 1.2 0 0 1 6.4 11.4 L11.6 6.2" />
  {:else if kind === "image"}
    <rect x="3" y="4" width="14" height="12" rx="1.5" />
    <circle cx="7.5" cy="8.5" r="1.4" />
    <path d="M3.5 14.5 L8 10.5 L11 13 L13.5 10.5 L16.5 13.5" />
  {:else if kind === "selection"}
    <path d="M4 4.5 V15.5" />
    <path d="M8 6.5 H16 M8 10 H16 M8 13.5 H12.5" />
  {:else if kind === "block"}
    <path d="M4 5 H16 M4 9 H16 M4 13 H11" />
    <path d="M13.5 12 L16.5 15 L13.5 18" />
  {:else if kind === "view"}
    <rect x="3" y="3.5" width="14" height="13" rx="1.5" />
    <path d="M3 8 H17 M8 8 V16.5 M12.5 8 V16.5" />
  {:else if kind === "page"}
    <circle cx="10" cy="10" r="7" />
    <path d="M3 10 H17 M10 3 C7.4 5.6 7.4 14.4 10 17 M10 3 C12.6 5.6 12.6 14.4 10 17" />
  {:else if kind === "skill"}
    <!-- a compass: the method that gives the turn its direction -->
    <circle cx="10" cy="10" r="7" />
    <path d="M13 7 L11.3 11.3 L7 13 L8.7 8.7 Z" />
  {:else}
    <path d="M3.5 4.5 A1 1 0 0 1 4.5 3.5 H8 A2 2 0 0 1 10 5.5 V16.5 A2 2 0 0 0 8 14.5 H4.5 A1 1 0 0 1 3.5 13.5 Z" />
    <path d="M16.5 4.5 A1 1 0 0 0 15.5 3.5 H12 A2 2 0 0 0 10 5.5 V16.5 A2 2 0 0 1 12 14.5 H15.5 A1 1 0 0 0 16.5 13.5 Z" />
  {/if}
</svg>
