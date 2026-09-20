<script lang="ts">
  import { avatarById, AVATARS } from "$lib/avatars";

  type FaceSize = "sm" | "hero";

  let { avatarId, size = "sm" }: { avatarId: string; size?: FaceSize } = $props();

  const avatar = $derived(avatarById(avatarId) ?? AVATARS[0]);
  const hero = $derived.by(() => {
    switch (size) {
      case "hero":
        return true;
      case "sm":
        return false;
      default: {
        const _exhaustive: never = size;
        return _exhaustive;
      }
    }
  });
</script>

{#if avatar}
  {#if hero}
    <div
      class="rounded-full bg-surface p-[3px] shadow-card ring-1 ring-black/8 dark:bg-white/8 dark:shadow-none dark:ring-white/10"
      aria-hidden="true"
    >
      <div
        class="size-[94px] overflow-hidden rounded-full outline-1 -outline-offset-1 outline-black/10 dark:outline-white/10"
      >
        <img src={avatar.src} alt="" class="size-full object-cover" />
      </div>
    </div>
  {:else}
    <div
      class="size-8 shrink-0 overflow-hidden rounded-full outline-1 -outline-offset-1 outline-black/10 dark:outline-white/10"
      aria-hidden="true"
    >
      <img src={avatar.src} alt="" class="size-full object-cover" />
    </div>
  {/if}
{/if}
