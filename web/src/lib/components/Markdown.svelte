<script lang="ts">
  import { renderMarkdown } from "#lib/markdown.ts";
  import { describeReferences } from "#lib/references.ts";

  let { text, group }: { text: string; group: string } = $props();

  let element: HTMLElement | undefined = $state();
  const html = $derived(renderMarkdown(text, { group }));

  $effect(() => {
    // Again whenever the text changes.
    void html;
    if (element) describeReferences(element, group);
  });
</script>

<div class="document" bind:this={element}>
  <!-- Rendered from Markdown with raw HTML escaped. -->
  {@html html}
</div>
