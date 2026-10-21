<script lang="ts">
  import MachineSettings from "../MachineSettings.svelte";
</script>

<p>
  Your machines run your agents, only for you, with the Claude Code, Codex or OpenCode installed
  and signed in on each. They connect to the hub, and your credentials stay on them. A job goes
  to the machine you prefer, if it has the runner, or else to any that has it; with none online,
  it waits. If you administer a group, a machine of yours may also work for its workshop
  (<code>stemma machine workshop add</code>, or in the group's settings): there you choose the
  agents it offers the workshop, in order, and how much each may do in a day. Set one up from
  its command line, signed in with <code>stemma login</code>:
</p>
<span class="copy-block">
  <span class="copy-block__label">On the machine</span>
  <pre id="machine-commands">stemma machine add</pre>
  <stemma-copy-button for="machine-commands" label="Copy the commands"></stemma-copy-button>
</span>
<MachineSettings />
