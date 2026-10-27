<script lang="ts">
  import { type Snapshot, testSignIn } from "../api.ts";
  import type { MachineControls } from "../machine.svelte.ts";
  import { check, RUNNERS, setting } from "../settings.ts";
  import SettingField from "./SettingField.svelte";

  let { controls, data }: { controls: MachineControls; data: Snapshot } = $props();

  const isolation = $derived(setting(data, "agents.isolation"));
  const contained = $derived(isolation !== undefined && isolation.value !== "none");
  const image = $derived(check(data, "Agents' image"));
  const imageSetting = $derived(setting(data, "agents.image"));
  /** An image of the person's own, which Stemma does not build. */
  const ownImage = $derived(imageSetting !== undefined && imageSetting.value !== data.stemma_image);
  /** The values being typed, by variable, until they are kept. */
  let typed = $state<Record<string, string>>({});

  /** What each agent said when tested, or why it could not. */
  let tested = $state<Record<string, { ok: boolean; text: string }>>({});
  let testing = $state("");

  async function test(agent: string) {
    testing = agent;
    try {
      tested[agent] = {
        ok: true,
        text: `It answered: ${await testSignIn(agent, controls.scope || null)}`,
      };
    } catch (error) {
      tested[agent] = { ok: false, text: String(error) };
    } finally {
      testing = "";
    }
  }

  const TOKEN = "CLAUDE_CODE_OAUTH_TOKEN";

  function keep(name: string) {
    const value = typed[name] ?? "";
    typed[name] = "";
    return controls.save({ kind: "secret", name, value }, "");
  }
</script>

{#snippet secret(
  name: string,
)}
  {@const kept = data.secrets.includes(name)}
  <div class="field">
    <span class="row">
      <label class="field__label grow" for="secret-{name}">
        <code>{name}</code>
        <span class="chip chip--small {kept ? "chip--success" : "chip--warning"}"
          >{kept ? "Kept" : "Not kept"}</span
        >
      </label>
      {#if kept}
        <button
          type="button"
          class="button button--quiet button--small"
          disabled={controls.saving(`secret:${name}`) || !!controls.scope}
          onclick={() => controls.save({ kind: "secret", name, value: null }, "")}
        >
          {#if controls.saving(`secret:${name}`)}
            <span class="spinner" aria-hidden="true"></span>
          {/if}
          Forget
        </button>
      {/if}
    </span>
    {#if controls.scope}
      <span class="field__hint">The same in every group.</span>
    {/if}
    <span class="row">
      <input
        id="secret-{name}"
        class="input grow"
        type="password"
        autocomplete="off"
        disabled={controls.saving(`secret:${name}`) || !!controls.scope}
        placeholder={kept ? "Replace its value" : "Its value"}
        bind:value={typed[name]}
      />
      <button
        type="button"
        class="button"
        disabled={controls.saving(`secret:${name}`) || !!controls.scope || !typed[name]}
        onclick={() => keep(name)}
      >
        {#if controls.saving(`secret:${name}`)}
          <span class="spinner" aria-hidden="true"></span>
        {/if}
        Keep
      </button>
    </span>
  </div>
{/snippet}

<p class="quiet">
  With a container, an answer runs with no limits inside one of its own, which holds only the
  group's workspace and the agent's own sign-in. Without, it runs here and only reads: it cannot run
  commands.
</p>
<ul class="card row-list">
  {#if isolation}
    <SettingField {controls} setting={isolation} description="Where answers run." />
  {/if}
  {#if contained && imageSetting}
    <li>
      <details class="row-list__item">
        <summary>
          <span class="row-list__text">
            <span class="row-list__name">Image</span>
            <span class="row-list__description identifier">{imageSetting.value}</span>
            {#if image?.remedy}
              <span class="row-list__description">{image.remedy}</span>
            {/if}
          </span>
          <span class="row-list__control">
            <!-- While it builds, an image already there says nothing of the new one. -->
            {#if controls.saving("image:")}
              <span class="chip chip--small chip--info chip--live"
                ><span class="chip__dot" aria-hidden="true"></span>Building</span
              >
            {:else if !image || controls.checking}
              <!-- Not checked yet, or being checked again after a change: the
                   state from before would say what is no longer known. -->
              {#if controls.checking}
                <span class="spinner" role="status" aria-label="Checking the image"></span>
              {/if}
            {:else if image.status === "success"}
              <span class="chip chip--small chip--success">{ownImage ? "Ready" : "Built"}</span>
            {:else}
              <span class="chip chip--small chip--warning"
                >{ownImage ? "Not ready" : "Not built"}</span
              >
            {/if}
          </span>
        </summary>
        <div class="row-list__body">
          {#if ownImage}
            <p class="quiet">
              Your own image: Stemma does not build it. It needs
              <span class="identifier">stemma</span>
              and the agents' programs; the checks say if something is missing. Go back to Stemma's
              image with the reset beside its name.
            </p>
            <SettingField {controls} setting={imageSetting} as="field" name="Your own image" />
          {:else}
            <p class="quiet">
              Stemma's image has the agents, Lean, LaTeX, git and this version of
              <span class="identifier">stemma</span>. Build it again after Stemma is updated.
            </p>
            <button
              type="button"
              class="button"
              disabled={controls.saving("image:")}
              onclick={() =>
                controls.save({ kind: "image", group: controls.scope || null }, "Images built.")}
            >
              {#if controls.saving("image:")}
                <span class="spinner" aria-hidden="true"></span>
              {/if}
              Build images
            </button>
            <details class="disclosure">
              <summary>Use your own image</summary>
              <div class="stack">
                <p class="quiet">
                  An image on this computer, built from Stemma's (<span class="identifier"
                    >FROM {data.stemma_image}</span
                  >) with what you add. Stemma does not build it, and checks that it has
                  <span class="identifier">stemma</span>
                  and the agents' programs.
                </p>
                <SettingField {controls} setting={imageSetting} as="field" name="Your own image" />
              </div>
            </details>
          {/if}
        </div>
      </details>
    </li>
  {/if}
</ul>

{#if contained}
  <section class="stack" aria-label="Sign-ins">
    <h2 class="label">Each agent's sign-in, in its container</h2>
    <ul class="card row-list">
      {#each RUNNERS as runner (runner.key)}
        {@const signIn = check(data, `${runner.name} in a container`)}
        {@const env = setting(data, `agents.${runner.key}.container.env`)}
        {@const names = Array.isArray(env?.value) ? (env.value as string[]) : []}
        <li>
          <details class="row-list__item">
            <summary>
              <span class="row-list__text">
                <span class="row-list__name">{runner.name}</span>
                <span class="row-list__description"
                  >{signIn?.status === "success"
                    ? signIn.detail
                    : (signIn?.remedy ?? "Not used here")}</span
                >
              </span>
              <span class="row-list__control">
                {#if controls.checking}
                  <span class="spinner" role="status" aria-label="Checking its sign-in"></span>
                {:else if signIn?.status === "success"}
                  <span class="chip chip--small chip--success">Ready</span>
                {:else if signIn}
                  <span class="chip chip--small chip--warning">Needs a sign-in</span>
                {/if}
              </span>
            </summary>
            <div class="row-list__body">
              {#if runner.key === "claude"}
                <p class="quiet">
                  It signs in with a token: run <code>claude setup-token</code> in a terminal, and
                  keep what it prints here.
                </p>
                {@render secret(TOKEN)}
              {:else}
                {@const own = data.signed_in.includes(runner.key)}
                <p class="quiet">
                  {own
                    ? `Signed in for its containers, apart from your own ${runner.name}: answers renew it, one at a time.`
                    : runner.key === "codex"
                      ? "It signs in for its containers apart from your own Codex: your browser opens, and you finish there."
                      : "It signs in for its containers apart from your own OpenCode, choosing a provider: run stemma machine sign-in opencode in a terminal."}
                </p>
                <span class="row">
                  {#if own}
                    <button
                      type="button"
                      class="button"
                      disabled={controls.saving(`sign_out:`) || !!controls.scope}
                      onclick={() => controls.save({ kind: "sign_out", agent: runner.key }, "")}
                    >
                      Forget
                    </button>
                  {:else if runner.key === "codex"}
                    <button
                      type="button"
                      class="button button--primary"
                      disabled={controls.saving("sign_in:") || !!controls.scope}
                      onclick={() => controls.save({ kind: "sign_in", agent: runner.key }, "")}
                    >
                      {#if controls.saving("sign_in:")}
                        <span class="spinner" aria-hidden="true"></span>
                      {/if}
                      Sign in
                    </button>
                  {/if}
                </span>
              {/if}
              <span class="row">
                <button
                  type="button"
                  class="button"
                  disabled={!!testing || !signIn || signIn.status !== "success"}
                  onclick={() => test(runner.key)}
                >
                  {#if testing === runner.key}
                    <span class="spinner" aria-hidden="true"></span>
                  {/if}
                  Test
                </button>
                <span class="quiet">Asks it for one word, in a container as an answer's.</span>
              </span>
              {#if tested[runner.key]}
                <p
                  class="message {tested[runner.key].ok ? "message--success" : "message--danger"}"
                  role="status"
                >
                  <span class="message__text">{tested[runner.key].text}</span>
                </p>
              {/if}
              <details class="disclosure">
                <summary>Other ways to sign in</summary>
                <div class="stack">
                  {#if env}
                    <SettingField
                      {controls}
                      setting={env}
                      as="field"
                      description="Variables, by name, one per line; their values are kept below, never in the settings."
                    />
                  {/if}
                  {#each names.filter(
                    (name) => runner.key !== "claude" || name !== TOKEN,
                  ) as name (name)}
                    {@render secret(name)}
                  {/each}
                  {#each ["files", "command"] as key (key)}
                    {@const value = setting(data, `agents.${runner.key}.container.${key}`)}
                    {#if value}
                      <SettingField
                        {controls}
                        setting={value}
                        as="field"
                        description={key === "files"
                          ? "One per line: the file here, or the file here → where it goes in the container."
                          : "The agent's program in the image."}
                      />
                    {/if}
                  {/each}
                </div>
              </details>
            </div>
          </details>
        </li>
      {/each}
    </ul>
  </section>
{/if}
