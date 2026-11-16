<script lang="ts">
  import Card from '../../components/ui/Card.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import AdvisorList, { type AdvisorListProps } from './AdvisorList.svelte';

  let props: AdvisorListProps = $props();
</script>

<Card title="Advisor" hint={advisor.loaded ? 'LOCAL MODEL' : 'RULES'} class="advisor">
  <AdvisorList {...props} />
</Card>
