<script lang="ts">
  import { renderMarkdown } from "#lib/markdown.ts";
  import { describeReferences } from "#lib/references.ts";

  interface Props {
    text: string;
    group: string;
    /** Whether headings are a paper's sections, with their slugs as ids. */
    sections?: boolean;
    /** Lines whose blocks are shown as changed. */
    changed?: Set<number>;
  }

  let { text, group, sections = false, changed }: Props = $props();

  let element: HTMLElement | undefined = $state();
  const html = $derived(renderMarkdown(text, { group, sections, changed }));

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
