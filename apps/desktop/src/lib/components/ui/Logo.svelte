<script module lang="ts">
  /** The two shapes of the Slice mark, in a 48 by 48 box. The opening screen draws the same ones. */
  export const LOGO_DISK_PATH = 'M43.02,17.82A20,20 0 1 1 28.16,4.44L25.46,17.15A7,7 0 1 0 30.66,21.84Z';
  export const LOGO_PIECE_PATH = 'M32.77,6.02A20,20 0 0 1 40.96,13.40L29.94,20.29A7,7 0 0 0 27.07,17.71Z';
</script>

<script lang="ts">
  interface Props {
    size?: number;
  }

  let { size = 26 }: Props = $props();
</script>

<!-- The Slice mark: a disk with one piece lifted out. -->
<svg class="logo" width={size} height={size} viewBox="0 0 48 48" aria-hidden="true">
  <path d={LOGO_DISK_PATH} fill="var(--logo-ring)" />
  <path d={LOGO_PIECE_PATH} fill="var(--accent)" transform="translate(3.4,-3.4)" />
</svg>

<style>
  .logo {
    flex: none;
  }
</style>
