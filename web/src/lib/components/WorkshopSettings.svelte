<script lang="ts">
  import { api, type WorkshopSettings } from "#lib/api.ts";
  import { toastError } from "#lib/toast.ts";

  let { group, admin }: { group: string; admin: boolean } = $props();

  let settings = $state<WorkshopSettings | null>(null);
  let from = $state("");
  let to = $state("");
  let zone = $state("");

  function show(found: WorkshopSettings) {
    settings = found;
    from = found.night_from;
    to = found.night_to;
    zone = found.time_zone;
  }

  $effect(() => {
    api.workshop(group).then(show).catch(toastError);
  });

  async function save(event: Event) {
    event.preventDefault();
    try {
      show(await api.updateWorkshop(group, { night_from: from, night_to: to, time_zone: zone }));
    } catch (error) {
      toastError(error);
    }
  }

  async function lend(value: boolean) {
    try {
      show(await api.updateWorkshop(group, { lend: value }));
    } catch (error) {
      toastError(error);
      api.workshop(group).then(show).catch(() => {});
    }
  }

  /** The time zone this browser is in, to offer it. */
  const here = Intl.DateTimeFormat().resolvedOptions().timeZone;
</script>

{#if settings}
  {#if admin}
    <form class="card card__body form" onsubmit={save}>
      <div class="row night-window">
        <div class="field grow">
          <label class="field__label" for="night-from">Night from</label>
          <input id="night-from" class="input" type="time" bind:value={from} required />
        </div>
        <div class="field grow">
          <label class="field__label" for="night-to">Until</label>
          <input id="night-to" class="input" type="time" bind:value={to} required />
        </div>
        <div class="field grow">
          <label class="field__label" for="night-zone">Time zone</label>
          <input id="night-zone" class="input input--mono" bind:value={zone} placeholder={here} required list="night-zones" />
          <datalist id="night-zones"><option value={here}></option><option value="UTC"></option></datalist>
        </div>
      </div>
      <div class="row">
        <span class="quiet grow">{settings.night_now ? "It is night now." : "It is not night now."}</span>
        <button type="submit" class="button button--primary">Save</button>
      </div>
    </form>
  {:else}
    <p>
      The night is from {settings.night_from} to {settings.night_to} ({settings.time_zone}){settings.night_now
        ? ", and it is night now"
        : ""}.
    </p>
  {/if}
  <label class="choice">
    <input type="checkbox" role="switch" aria-checked={settings.lend} checked={settings.lend} onchange={(e) => lend(e.currentTarget.checked)} />
    <span>
      Lend my own worker to the workshop at night: with Lean and <code>[workshop]</code> in its
      configuration, it works on the group's tasks with my own runners and subscription.
    </span>
  </label>
  {#if settings.lenders.length}
    <p class="quiet">
      Lent at night: {settings.lenders.map((u) => u.display_name).join(", ")}.
    </p>
  {/if}
{:else}
  <span class="spinner" role="status" aria-label="Loading"></span>
{/if}
