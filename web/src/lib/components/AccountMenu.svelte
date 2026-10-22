<script lang="ts">
  import { ask } from "#lib/ask.svelte.ts";
  import { goto } from "$app/navigation";
  import { session, signOut } from "#lib/session.svelte.ts";
  import { openSettings } from "#lib/settings.svelte.ts";
  import { toastError } from "#lib/toast.ts";
  import Avatar from "./Avatar.svelte";
  import Glyph from "./Glyph.svelte";
  let { role }: { role?: string } = $props();
  let leaving = $state(false);
  async function leave() {
    if (__STEMMA_DESKTOP__ && !await ask({ title: "Log out?", text: "This app and the command line on this machine are both signed out of the hub.", action: "Log out" })) return;
    leaving = true;
    try { await signOut(); await goto("/"); } catch (error) { toastError(error); } finally { leaving = false; }
  }
</script>

{#if session.me}
  <stemma-menu match-width>
    <button type="button" class="account account--menu" disabled={leaving} aria-label="Your account, {session.me.display_name}">
      <Avatar user={session.me} />
      <span class="account__details"><span class="account__name">{session.me.display_name}</span></span>
      {#if leaving}<span class="spinner" aria-hidden="true"></span>{/if}
      <Glyph name="chevron-down" class="account__chevron" small />
    </button>
    <div class="menu">
      <div class="menu__header">
        <Avatar user={session.me} />
        <div class="menu__identity">
          <span class="menu__name">{session.me.display_name}</span>
          <span class="menu__meta">@{session.me.username}{role ? `, ${role}` : ""}</span>
        </div>
      </div>
      <hr class="menu__separator" />
      <button type="button" class="menu__item" onclick={() => openSettings("Profile")}><Glyph name="person" small /> Profile</button>
      <button type="button" class="menu__item" onclick={() => openSettings("Appearance")}><Glyph name="appearance" small /> Appearance</button>
      <button type="button" class="menu__item" onclick={() => openSettings()}><Glyph name="settings" small /> Settings</button>
      {#if session.me.hub_admin}
        <hr class="menu__separator" />
        <a class="menu__item" href="/hub"><Glyph name="shield" small /><span class="grow">Hub administration</span><Glyph name="arrow-right" small /></a>
      {/if}
      <hr class="menu__separator" />
      <button type="button" class="menu__item" onclick={leave}><Glyph name="log-out" small /> Log out</button>
    </div>
  </stemma-menu>
{/if}
