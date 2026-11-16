import { formatSize, type SizeUnits } from '../format';
import { percent } from './mock-scan-question';
import type { MockLabel, RuleView } from './mock-suggestions';
import type { MockNode, MockTree } from './mock-tree';
import type { AdvisorAnswerDto, SummaryDto } from './types';

/*
 * What `askAdvisor` says about one folder, as `commands/advisor/folder.rs` writes it: from Platter's rules alone, never
 * from a model. The order is the guard, a rule that covers the folder (the built-in table or the user's own), a label
 * the model made earlier in this scan, then "no rule". Nothing the model said about a folder is in the answer, only
 * which way it marked it. Every answer ends with the folder's size and how much of it is older than a year.
 */

const NO_RULE = 'Platter has no rule for it; deleting it is your call.';
const MARKED_FOR_REVIEW = 'The local model marked it for review.';
const MARKED_TO_KEEP = 'The local model marked it to keep.';
const LEFT_ALONE = 'Left alone by Platter.';
const UNREADABLE = 'Platter could not read it, so its size is unknown.';

export interface FolderFacts {
  tree: MockTree;
  node: MockNode;
  /** The folder's path as the answer shows it. */
  path: string;
  view: RuleView;
  /** Why the guard leaves the folder alone, or `null`. */
  guard: string | null;
  /** The labels the model made earlier in this scan. */
  labels: readonly MockLabel[];
  units: SizeUnits;
  summary(node: MockNode): SummaryDto;
}

/** How much of the folder is older than a year; nothing for an empty folder. */
function olderThanAYear(facts: FolderFacts): string | undefined {
  const { node, units } = facts;
  const size = node.allocated;

  if (size === 0) {
    return undefined;
  }

  const older = facts
    .summary(node)
    .byAge.filter((stat) => stat.bucket === 'twoYears' || stat.bucket === 'older')
    .reduce((bytes, stat) => bytes + stat.allocated, 0);
  const share = percent(older, size);
  const bytes = formatSize(older, units);

  if (older === 0) {
    return 'None of it is older than a year.';
  }

  return share === 0
    ? `Less than 1% of it (${bytes}) is older than a year.`
    : `${share}% of it (${bytes}) is older than a year.`;
}

/** Who decided (a rule's title), then the size and the share older than a year. */
function detailOf(facts: FolderFacts, who?: string): string {
  const parts = who === undefined ? [] : [who];

  if (facts.node.unreadable) {
    return [...parts, UNREADABLE].join(' ');
  }

  parts.push(`${formatSize(facts.node.allocated, facts.units)}.`);

  const older = olderThanAYear(facts);

  if (older !== undefined) {
    parts.push(older);
  }

  return parts.join(' ');
}

export function folderAnswer(facts: FolderFacts): AdvisorAnswerDto {
  const { node, path, view, guard } = facts;
  const reply = (
    verdict: AdvisorAnswerDto['verdict'],
    reason: string,
    source: AdvisorAnswerDto['source'],
    who?: string,
  ): AdvisorAnswerDto => ({ node: node.id, path, verdict, reason, source, detail: detailOf(facts, who) });

  if (guard !== null) {
    return reply('keep', `${LEFT_ALONE} ${guard}`, 'guard');
  }

  const cover = view.covering(node);

  if (cover?.source === 'user') {
    return reply(cover.verdict, cover.reason, 'user', `Your rule: ${cover.title}.`);
  }

  if (cover) {
    return reply(cover.verdict, cover.reason, 'rule', `Rule: ${cover.title}.`);
  }

  const label = facts.labels.find((entry) => entry.node === node.id);

  if (!label) {
    return reply('review', NO_RULE, 'unruled');
  }

  const marked = label.verdict === 'keep' ? MARKED_TO_KEEP : MARKED_FOR_REVIEW;

  return reply(label.verdict, `${NO_RULE} ${marked}`, 'model');
}
