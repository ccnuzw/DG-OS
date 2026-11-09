import { t } from './index';

const categoryKeys: Record<string, string> = {
  writing: 'skills.category.writing',
  visual: 'skills.category.visual',
  'video-audio': 'skills.category.video-audio',
  'documents-office': 'skills.category.documents-office',
  'data-research': 'skills.category.data-research',
  'development-automation': 'skills.category.development-automation',
};

export function skillCategoryLabel(category: string): string {
  const key = categoryKeys[category];
  return key ? t(key) : category;
}
