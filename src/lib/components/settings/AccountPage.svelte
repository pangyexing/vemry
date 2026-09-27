<script lang="ts">
  import AccountPanel from "../AccountPanel.svelte";
  import SettingRow from "./SettingRow.svelte";
</script>

<!-- The account is one card of its own (sign-in, entitlements); in search results it
     is a row that leads here -->
<SettingRow id="account.panel" block>
  <AccountPanel />
</SettingRow>
