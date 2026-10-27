<script lang="ts">
  import { api, type NotificationSettings } from "#lib/api.ts";
  import Loading from "#lib/components/Loading.svelte";
  import { scriptedSelect } from "#lib/scripted-select.ts";
  import { toastError } from "#lib/toast.ts";

  const LIMITS = ["", "5", "20", "60"];
  const KEEP = ["", "7", "30", "365"];

  let settings = $state<NotificationSettings | null>(null);
  let saving = $state(false);
  /** The limit's choice: "" for none, a preset, or "custom". */
  let limit = $state("");
  let count = $state(10);

  $effect(() => {
    api
      .notificationSettings()
      .then((found) => {
        settings = found;
        const value = found.notes_per_hour === null ? "" : String(found.notes_per_hour);
        limit = LIMITS.includes(value) ? value : "custom";
        if (found.notes_per_hour !== null) count = found.notes_per_hour;
      })
      .catch(toastError);
  });

  async function save(change: Partial<NotificationSettings>) {
    if (!settings) return;
    saving = true;
    try {
      settings = await api.updateNotificationSettings({ ...settings, ...change });
    } catch (error) {
      toastError(error);
    } finally {
      saving = false;
    }
  }

  function chooseLimit(value: string) {
    limit = value;
    if (value === "custom") return void save({ notes_per_hour: count });
    void save({ notes_per_hour: value ? Number(value) : null });
  }
</script>

{#if settings === null}
  <Loading label="Loading your notification settings" />
{:else}
  <p class="quiet">
    Mentions, your agents' answers and what happens to your groups always reach you. Notes are
    what people and their agents choose to leave you: say what you accept.
  </p>
  <ul class="card row-list">
    <li class="row-list__item">
      <span class="row-list__text">
        <label class="row-list__name" for="notes-per-hour">Notes an hour</label>
        <span class="row-list__description">More than this, and the sender is told to wait.</span>
      </span>
      <span class="row-list__control">
        {#if limit === "custom"}
          <stemma-stepper>
            <input
              type="number"
              aria-label="Notes an hour"
              min="1"
              step="1"
              bind:value={count}
              disabled={saving}
              onchange={() => count >= 1 && save({ notes_per_hour: count })}
            />
          </stemma-stepper>
        {/if}
        <stemma-select
          ><select
            use:scriptedSelect={[limit, saving]}
            id="notes-per-hour"
            class="input"
            value={limit}
            disabled={saving}
            onchange={(event) => chooseLimit(event.currentTarget.value)}
          >
            <option value="">No limit</option>
            <option value="5">5</option>
            <option value="20">20</option>
            <option value="60">60</option>
            <option value="custom">A number…</option>
          </select></stemma-select
        >
      </span>
    </li>
    <li class="row-list__item">
      <span class="row-list__text">
        <label class="row-list__name" for="notes-from-agents">Notes from agents</label>
        <span class="row-list__description">
          Accept notes that people's agents leave, not only people's own.
        </span>
      </span>
      <span class="row-list__control">
        <input
          id="notes-from-agents"
          type="checkbox"
          role="switch"
          aria-checked={settings.notes_from_agents}
          checked={settings.notes_from_agents}
          disabled={saving}
          onchange={(event) => save({ notes_from_agents: event.currentTarget.checked })}
        />
      </span>
    </li>
    <li class="row-list__item">
      <span class="row-list__text">
        <label class="row-list__name" for="keep-read">Keep read notifications</label>
        <span class="row-list__description">Unread ones are always kept.</span>
      </span>
      <span class="row-list__control">
        <stemma-select
          ><select
            use:scriptedSelect={[settings.keep_read_days, saving]}
            id="keep-read"
            class="input"
            value={settings.keep_read_days === null ? "" : String(settings.keep_read_days)}
            disabled={saving}
            onchange={(event) =>
              save({ keep_read_days: event.currentTarget.value ? Number(event.currentTarget.value) : null })}
          >
            {#if settings.keep_read_days !== null && !KEEP.includes(String(settings.keep_read_days))}
              <option value={String(settings.keep_read_days)}>{settings.keep_read_days} days</option>
            {/if}
            <option value="">Always</option>
            <option value="7">A week</option>
            <option value="30">A month</option>
            <option value="365">A year</option>
          </select></stemma-select
        >
      </span>
    </li>
  </ul>
{/if}
