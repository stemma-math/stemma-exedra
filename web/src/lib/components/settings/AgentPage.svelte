<script lang="ts">
  import { session } from "#lib/session.svelte.ts";
  import PersonalAgentSettings from "../PersonalAgentSettings.svelte";
</script>

<p>
  Your agents answer for you, in every group you are in: when you write <code>@claude</code>,
  your Claude answers, on your machine, with your own sign-in, and its message is signed
  “Claude, for {session.me?.display_name}”. You have one for each runner, and may add more, such
  as <code>@claude-sonnet</code> with another model.
</p>
<PersonalAgentSettings />
