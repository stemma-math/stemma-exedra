<script lang="ts">
  import { page } from "$app/state";
</script>

<main class="page page--narrow">
  <h1>{page.status === 404 ? "Not here" : "Something went wrong"}</h1>
  <p>{page.error?.message}</p>
  <p><a class="button button--link" href="/">Go home</a></p>
</main>
