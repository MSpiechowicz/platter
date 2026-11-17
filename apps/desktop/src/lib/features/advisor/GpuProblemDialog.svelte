<script lang="ts">
  import Button from '../../components/ui/Button.svelte';
  import Dialog from '../../components/ui/Dialog.svelte';
  import { advisor } from '../../state/advisor.svelte';
  import { visibleText } from '../overview/display';
  import { gpuProblemText } from './model';

  /**
   * Tells the user that the graphics card failed while the advisor ran, and what Platter did about it. Escape or a
   * click outside closes it without an answer: the card stays blocked and the question is not asked again this
   * session (the next start asks it again).
   */
  const problem = $derived(advisor.gpuProblem);
</script>

<Dialog
  open={problem !== undefined}
  title="The graphics card stopped working"
  wide
  description={problem ? visibleText(gpuProblemText(problem)) : undefined}
  onclose={() => advisor.dismissGpuProblem()}
>
  {#snippet actions()}
    <Button onclick={() => advisor.answerGpuProblem('tryAgain')}>Try the card again next time</Button>
    <Button data-autofocus variant="primary" onclick={() => advisor.answerGpuProblem('useCpu')}>
      Use the processor from now on
    </Button>
  {/snippet}
</Dialog>
