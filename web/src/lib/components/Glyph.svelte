<script lang="ts">
  import glyphs from "@stemma-math/design/glyphs.svg?url";

  let {
    name,
    small = false,
    label,
    class: extra,
  }: { name: string; small?: boolean; label?: string; class?: string } = $props();
</script>

<svg
  class={["glyph", small && "glyph--small", extra]}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <use href="{glyphs}#{name}" />
</svg>
