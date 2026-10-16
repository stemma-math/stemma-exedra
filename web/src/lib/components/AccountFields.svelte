<script lang="ts">
  let {
    username = $bindable(""),
    displayName = $bindable(""),
    password = $bindable(""),
  }: { username: string; displayName: string; password: string } = $props();
</script>

<div class="field">
  <label class="field__label" for="account-name">Your name</label>
  <input id="account-name" class="input" bind:value={displayName} autocomplete="name" required />
  <span class="field__hint">As others in the group see it.</span>
</div>
<div class="field">
  <label class="field__label" for="account-username">Username</label>
  <input
    id="account-username"
    class="input input--mono"
    bind:value={username}
    autocomplete="username"
    autocapitalize="none"
    pattern="[a-z0-9][a-z0-9._\-]{'{0,31}'}"
    required
  />
  <span class="field__hint">To sign in: lowercase letters, digits, dots, dashes or underscores.</span>
</div>
<div class="field">
  <label class="field__label" for="account-password">Password</label>
  <input
    id="account-password"
    class="input"
    type="password"
    bind:value={password}
    autocomplete="new-password"
    minlength="8"
    required
  />
  <span class="field__hint">At least 8 characters.</span>
</div>
