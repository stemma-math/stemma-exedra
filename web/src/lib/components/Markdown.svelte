<script lang="ts">
  import { renderMarkdown } from "#lib/markdown.ts";
  import { describeReferences } from "#lib/references.ts";

  interface Props {
    text: string;
    missing?: string[];
    group: string;
  }

  let { text, group, missing = [] }: Props = $props();

  let element: HTMLElement | undefined = $state();
  const html = $derived(renderMarkdown(text, { group, missing }));

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
