import type { Category } from './api/types';

export interface CategoryMeta {
  label: string;
  /** CSS colour, a variable from app.css. */
  color: string;
}

/** Display name and colour of every file category. Green is not used: it means "safe". */
export const CATEGORY_META: Record<Category, CategoryMeta> = {
  video: { label: 'Video', color: 'var(--cat-video)' },
  image: { label: 'Images', color: 'var(--cat-image)' },
  audio: { label: 'Audio', color: 'var(--cat-audio)' },
  document: { label: 'Documents', color: 'var(--cat-document)' },
  archive: { label: 'Archives', color: 'var(--cat-archive)' },
  code: { label: 'Code', color: 'var(--cat-code)' },
  build: { label: 'Build artifacts', color: 'var(--cat-build)' },
  package: { label: 'Packages', color: 'var(--cat-package)' },
  app: { label: 'Apps & games', color: 'var(--cat-app)' },
  diskImage: { label: 'Disk images', color: 'var(--cat-diskImage)' },
  data: { label: 'Data & models', color: 'var(--cat-data)' },
  other: { label: 'Other', color: 'var(--cat-other)' },
};
