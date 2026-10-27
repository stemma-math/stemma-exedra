<script lang="ts">
  import { renderMarkdown } from "#lib/markdown.ts";
  import { describeReferences } from "#lib/references.ts";

  interface Props {
    text: string;
    missing?: string[];
    group: string;
    /** A short text among interface, at its size. */
    compact?: boolean;
  }

  let { text, group, missing = [], compact = false }: Props = $props();

  let element: HTMLElement | undefined = $state();
  const html = $derived(renderMarkdown(text, { group, missing }));

  $effect(() => {
    // Again whenever the text changes.
    void html;
    if (element) describeReferences(element, group);
  });
</script>

<div class="document" class:document--compact={compact} bind:this={element}>
  <!-- Rendered from Markdown with raw HTML escaped. -->
  {@html html}
</div>
