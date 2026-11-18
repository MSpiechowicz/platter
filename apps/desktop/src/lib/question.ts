/*
 * A free question as the backend takes it (`clean_question`): control characters, invisible formatting (bidi marks,
 * zero-width) and line or paragraph separators count as spaces, so a question of only those is blank.
 */

/** The characters of the backend's `is_invisible` (`crates/platter-ai/src/facts.rs`), range for range. */
const INVISIBLE =
  /[\p{Cc}\u00AD\u0600-\u0605\u061C\u06DD\u070F\u08E2\u180E\u200B-\u200F\u2028\u2029\u202A-\u202E\u2060-\u206F\uFEFF\uFFF9-\uFFFB\u{110BD}\u{110CD}\u{1BCA0}-\u{1BCA3}\u{1D173}-\u{1D17A}\u{E0001}\u{E0020}-\u{E007F}]/gu;

/** Whether `question` holds nothing the backend would read: only white space and invisible characters. */
export function isBlankQuestion(question: string): boolean {
  return question.replace(INVISIBLE, ' ').trim() === '';
}

/** The question as the backend keeps it: invisible characters as spaces, white space collapsed, trimmed. */
export function cleanQuestion(question: string): string {
  return question.replace(INVISIBLE, ' ').split(/\s+/u).filter(Boolean).join(' ');
}
