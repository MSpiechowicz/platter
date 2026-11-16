<script lang="ts">
  import Card from '../../components/ui/Card.svelte';
  import AdvisorList, { type AdvisorListProps } from './AdvisorList.svelte';

  let props: AdvisorListProps = $props();
</script>

<Card title="Advisor" hint="RULES" class="advisor">
  <AdvisorList {...props} />
</Card>
