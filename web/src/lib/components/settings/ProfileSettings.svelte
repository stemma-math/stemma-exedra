<script lang="ts">
  import NameField from "#lib/components/NameField.svelte";
  import { api } from "#lib/api.ts";
  import { session, setSession } from "#lib/session.svelte.ts";
  import { toast, toastError } from "#lib/toast.ts";
  import { connection } from "#lib/transport.svelte.ts";

  let username = $state(session.me?.username ?? "");
  let available = $state(false);
  let displayName = $state(session.me?.display_name ?? "");
  let current = $state("");
  let next = $state("");
  let savingName = $state(false);
  let changingPassword = $state(false);

  async function saveName(event: Event) {
    event.preventDefault();
    savingName = true;
    try {
      setSession(await api.updateMe(displayName, username));
      toast("Your name is saved.", "success");
    } catch (error) {
      toastError(error);
    } finally { savingName = false; }
  }

  async function changePassword(event: Event) {
    event.preventDefault();
    changingPassword = true;
    try {
      await api.changePassword(current, next);
      current = next = "";
      toast("Your password is changed. Other sessions are signed out.", "success");
    } catch (error) {
      toastError(error);
    } finally { changingPassword = false; }
  }
</script>

<form class="setting form" onsubmit={saveName}>
  <NameField id="username" label="Username" kind="user" bind:value={username} bind:available />
  <div class="field">
    <label class="field__label" for="display-name">Your name</label>
    <input id="display-name" class="input" bind:value={displayName} required maxlength="80" />
  </div>
  <p class="quiet">Signed in as <span class="identifier">{session.me?.username}</span>{#if __STEMMA_DESKTOP__ && connection.hub}, at <span class="identifier">{connection.hub}</span>{/if}.</p>
  <div class="row row--end"><button type="submit" class="button" disabled={savingName || !available}>{#if savingName}<span class="spinner" aria-hidden="true"></span>{/if}Save</button></div>
</form>

<form class="setting form" onsubmit={changePassword}>
  <h2 class="label">Password</h2>
  <div class="field">
    <label class="field__label" for="current-password">Current password</label>
    <input
      id="current-password"
      class="input"
      type="password"
      bind:value={current}
      autocomplete="current-password"
      required
    />
  </div>
  <div class="field">
    <label class="field__label" for="new-password">New password</label>
    <input
      id="new-password"
      class="input"
      type="password"
      bind:value={next}
      autocomplete="new-password"
      minlength="8"
      required
    />
  </div>
  <div class="row row--end"><button type="submit" class="button" disabled={changingPassword}>{#if changingPassword}<span class="spinner" aria-hidden="true"></span>{/if}Change the password</button></div>
</form>
