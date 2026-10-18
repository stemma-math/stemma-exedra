<script lang="ts">
  import { onMount } from "svelte";
  import { action, login, openHub, RUNNER_NAMES, RUNNERS, type Snapshot, snapshot, type WorkshopAgent } from "./api.ts";

  let data = $state<Snapshot | null>(null);
  let busy = $state(false);
  let error = $state("");
  let notice = $state("");
  let url = $state("");
  let username = $state("");
  let password = $state("");
  let name = $state("This machine");
  let folder = $state("");
  let group = $state("");
  let runtime = $state("auto");
  let image = $state("");
  let source = $state("git");
  let agents = $state<WorkshopAgent[]>([]);
  let runner = $state<string>("claude-code");
  let credentialRunner = $state<string>("claude-code");
  let env = $state("");
  let files = $state("");
  const selectedGroup = $derived(data?.me?.groups.find((g) => g.slug === group));
  const admin = $derived(selectedGroup?.role === "admin");
  const lent = $derived(data?.machine?.workshop.includes(group) ?? false);

  async function load() {
    data = await snapshot();
    url = data.hub ?? "";
    username = data.username ?? "";
    name = data.name;
    folder = data.work_dir;
    runtime = data.runtime;
    image = data.image;
    agents = structuredClone($state.snapshot(data.machine?.workshop_agents ?? []));
    if (!data.me?.groups.some((g) => g.slug === group)) group = data.me?.groups[0]?.slug ?? "";
    loadCredentials();
  }
  function loadCredentials() {
    const credentials = data?.credentials[credentialRunner];
    env = credentials?.env?.join("\n") ?? "";
    files = credentials?.files?.map((f) => f.to ? `${f.from}=${f.to}` : f.from).join("\n") ?? "";
  }
  async function run(work: () => Promise<void>, message: string, reload = true) {
    busy = true; error = ""; notice = "";
    try { await work(); if (reload) await load(); notice = message; }
    catch (e) { error = String(e); try { data = await snapshot(); } catch { /* Keep the last known state when the backend is unavailable. */ } }
    finally { busy = false; }
  }
  function save(command: object, message = "Saved.") {
    return run(() => action(command), message);
  }
  function move(index: number, offset: number) {
    const next = agents.slice();
    [next[index], next[index + offset]] = [next[index + offset], next[index]];
    agents = next;
  }
  function addAgent() {
    agents = [...agents, { runner, model: null, args: [], daily_jobs: null, daily_minutes: null }];
  }
  onMount(() => { run(load, "", false); });
</script>

<main class="stack" aria-busy={busy}>
  <header class="row">
    <stemma-mark></stemma-mark>
    <div class="grow"><h1>This machine</h1><p class="quiet">Stemma · your agents and the group’s workshop</p></div>
    <button type="button" class="button" disabled={busy} onclick={() => run(load, "Refreshed.", false)}>Refresh</button>
    {#if data?.me}<button type="button" class="button button--primary" disabled={busy} onclick={() => run(openHub, "The hub opens in its own window. Sign in there if its browser session has expired.")}>Open Stemma</button>{/if}
  </header>
  {#if error}<p class="message message--danger" role="alert">{error}</p>{/if}
  {#if notice}<p class="message message--success" role="status">{notice}</p>{/if}
  {#if busy}<p class="quiet" role="status">Working… Image builds may take several minutes.</p>{/if}
  {#if data?.hub_error}<p class="message message--warning">{data.hub_error}</p>{/if}

  {#if !data?.me}
    <section class="card panel stack">
      <h2>Sign in to a hub</h2>
      <p class="quiet">Uses the same sign-in and settings as <code>stemma login</code>.</p>
      <form class="stack" onsubmit={(event) => { event.preventDefault(); run(async () => { try { await login(url, username, password); } finally { password = ""; } }, "Signed in."); }}>
        <label class="field">Hub address<input class="input" type="url" bind:value={url} placeholder="https://stemma.example.org" required /></label>
        <label class="field">Username<input class="input" autocomplete="username" bind:value={username} required /></label>
        <label class="field">Password<input class="input" type="password" autocomplete="current-password" bind:value={password} required /></label>
        <div><button type="submit" class="button button--primary" disabled={busy}>Sign in</button></div>
      </form>
    </section>
  {:else}
    <p class="quiet">Signed in as {data.me.username} · {data.hub}</p>
  {/if}

  {#if data}
    <div class="machine-columns">
      <section class="card panel stack" aria-labelledby="personal-heading">
        <div class="row"><h2 id="personal-heading" class="grow">Personal service</h2><span class:chip--success={data.personal} class="chip">{data.personal ? "On" : "Off"}</span></div>
        <p>Your agents, with your own sign-in, in all your groups.</p>
        {#each data.agents as agent (agent.runner)}
          <div class="row"><strong class="grow">{agent.name}</strong><span class="chip">{agent.sign_in}</span></div>
        {/each}
        <p class="quiet">Sign in with the agent’s own program. Refresh after installing or signing in.</p>
        {#if !data.registered}
          <form class="stack" onsubmit={(event) => { event.preventDefault(); save({ kind: "register", name, preferred: false }, "Machine registered. Turn on the service when ready."); }}>
            <label class="field">Machine name<input class="input" bind:value={name} required /></label>
            <div><button type="submit" class="button button--primary" disabled={busy || !data.me}>Register this machine</button></div>
          </form>
        {:else}
          <strong>{data.name}</strong>
          <form class="stack" onsubmit={(event) => { event.preventDefault(); save({ kind: "folder", path: folder }); }}>
            <label class="field">Repository copies<input class="input" bind:value={folder} required /></label>
            <div><button type="submit" class="button" disabled={busy}>Save folder</button></div>
          </form>
          <div class="row">
            <button type="button" class="button button--primary" disabled={busy || !data.me} onclick={() => save({ kind: "service", personal: !data?.personal, workshop: data?.workshop }, "Personal service updated.")}>{data.personal ? "Turn off" : "Turn on"}</button>
            <button type="button" class="button" disabled={busy || !data.machine} onclick={() => save({ kind: "prefer", preferred: !data?.machine?.preferred })}>{data.machine?.preferred ? "Remove preference" : "Set as preferred"}</button>
          </div>
          <p class="quiet">{data.machine?.online ? "Connected to the hub" : data.serving ? "Connecting to the hub…" : "Service stopped"}. Keep this app open while serving. The stemma CLI must be installed for personal agents’ tools.</p>
          <button type="button" class="button button--danger" disabled={busy || !data.machine} onclick={() => { if (confirm("Remove this machine and stop both services?")) save({ kind: "remove" }, "Machine removed."); }}>Remove this machine</button>
        {/if}
      </section>

      <section class="card panel stack" aria-labelledby="workshop-heading">
        <div class="row"><h2 id="workshop-heading" class="grow">The group’s workshop</h2><span class:chip--success={data.workshop} class="chip">{data.workshop ? "On" : "Off"}</span></div>
        <p>Work the group sends, in a separate container for each job.</p>
        <label class="field">Group<select class="input" bind:value={group}><option value="">Choose a group</option>{#each data.me?.groups ?? [] as g (g.slug)}<option value={g.slug}>{g.name}</option>{/each}</select></label>
        {#if !admin}<p class="message message--info">Only group admins can lend a machine to the workshop.</p>{/if}
        <div class="stack tools"><p><strong>Lean (elan)</strong> · {data.elan ?? "Not installed on this machine"}</p><p><strong>LaTeX</strong> · {data.latex ?? "Not installed on this machine"}</p><p class="quiet">The workshop image supplies Lean and LaTeX to isolated jobs.</p></div>
        <fieldset class="fieldset stack" disabled={busy || !data.registered}>
          <legend>Isolation</legend>
          <label class="field">Runtime<select class="input" bind:value={runtime}><option value="auto">Automatic</option><option value="podman">Podman</option><option value="docker">Docker</option>{#if runtime === "host"}<option value="host">Unisolated (configured in CLI)</option>{/if}</select></label>
          {#if runtime === "host"}<p class="message message--warning">This machine was configured to run unisolated. Choose Docker or Podman to isolate the workshop.</p>{/if}
          <label class="field">Image<input class="input" bind:value={image} required /></label>
          <div><button type="button" class="button" disabled={runtime === "host"} onclick={() => save({ kind: "containers", runtime, image })}>Save isolation</button></div>
          <label class="field">Stemma in the image<select class="input" bind:value={source}><option value="git">Build from release tag (any architecture)</option><option value="release">Download release (x86_64)</option></select></label>
          <div><button type="button" class="button" onclick={() => run(async () => { await action({ kind: "containers", runtime, image }); await action({ kind: "build", source }); }, "Workshop image built.")} disabled={runtime === "host"}>Build image</button></div>
        </fieldset>
        <fieldset class="fieldset stack" disabled={busy || !data.registered || !data.me}>
          <legend>Agents offered, in preference order</legend>
          <p class="quiet">Offers and daily limits apply to every workshop this machine serves. An empty limit means unlimited.</p>
          {#each agents as agent, index (index)}
            <div class="card panel stack">
              <strong>{index + 1}. {RUNNER_NAMES[agent.runner] ?? agent.runner}</strong>
              <label class="field">Model (optional)<input class="input" value={agent.model ?? ""} oninput={(e) => agent.model = e.currentTarget.value || null} /></label>
              <label class="field">Jobs per day<input class="input" type="number" min="0" step="1" value={agent.daily_jobs ?? ""} oninput={(e) => agent.daily_jobs = e.currentTarget.value === "" ? null : Number(e.currentTarget.value)} /></label>
              <label class="field">Minutes per day<input class="input" type="number" min="0" step="1" value={agent.daily_minutes ?? ""} oninput={(e) => agent.daily_minutes = e.currentTarget.value === "" ? null : Number(e.currentTarget.value)} /></label>
              <div class="row"><button type="button" class="button button--small" disabled={index === 0} onclick={() => move(index, -1)}>Earlier</button><button type="button" class="button button--small" disabled={index === agents.length - 1} onclick={() => move(index, 1)}>Later</button><button type="button" class="button button--small button--danger" onclick={() => agents = agents.filter((_, i) => i !== index)}>Remove</button></div>
            </div>
          {/each}
          <div class="row"><label class="field grow">Agent<select class="input" bind:value={runner}>{#each RUNNERS as r (r)}<option value={r}>{RUNNER_NAMES[r]}</option>{/each}</select></label><button type="button" class="button" onclick={addAgent}>Add agent</button></div>
          <div><button type="button" class="button" onclick={() => save({ kind: "agents", agents })}>Save offers</button></div>
        </fieldset>
        <details class="disclosure">
          <summary>Workshop sign-in</summary>
          <div class="stack panel">
            <p class="quiet">Choose only the credentials to share with isolated workshop jobs. Files are mounted read-only. Enter variable names, never their values. Personal sign-in is not shared automatically.</p>
            <label class="field">Agent<select class="input" bind:value={credentialRunner} onchange={loadCredentials}>{#each RUNNERS as r (r)}<option value={r}>{RUNNER_NAMES[r]}</option>{/each}</select></label>
            <label class="field">Environment variable names (one per line)<textarea class="input" bind:value={env}></textarea></label>
            <label class="field">Files (one per line: ~/path or /host/path=/container/path)<textarea class="input" bind:value={files}></textarea></label>
            <div><button type="button" class="button" disabled={busy || !data.registered} onclick={() => save({ kind: "credentials", runner: credentialRunner, env: env.split("\n").map((s) => s.trim()).filter(Boolean), files: files.split("\n").map((s) => s.trim()).filter(Boolean).map((s) => { const at = s.indexOf("="); return at < 0 ? { from: s } : { from: s.slice(0, at), to: s.slice(at + 1) }; }) })}>Save workshop sign-in</button></div>
          </div>
        </details>
        <div class="row">
          <button type="button" class="button" disabled={busy || !admin || !data.registered} onclick={() => save({ kind: "lend", group, enabled: !lent }, lent ? "Machine withdrawn from this workshop." : "Machine lent to this workshop.")}>{lent ? "Withdraw from this group" : "Lend to this group"}</button>
          <button type="button" class="button button--primary" disabled={busy || !data.registered || !data.me || (!data.workshop && !data.machine?.workshop.length)} onclick={() => save({ kind: "service", personal: data?.personal, workshop: !data?.workshop }, "Workshop service updated.")}>{data.workshop ? "Turn off workshop" : "Turn on workshop"}</button>
        </div>
        <p class="quiet">Turning off pauses workshop work for every group served here. Settings changes reconnect this machine and requeue active work.</p>
      </section>
    </div>
  {/if}
</main>
