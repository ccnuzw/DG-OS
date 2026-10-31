import { cn } from '@dgos/ui';
import { FOCUS } from './ChatNotice';

// Mandatory disclosures of FR-006 (D072, D074, D075, D022). Open by default so they are visible on the page.

export const DELETE_CONFIRM_TEXT = '确定删除？应用内将无法恢复。已删除内容可能残留在数据库空闲页和备份中。';

export const DISCLOSURES: readonly string[] = [
  '聊天正文保存在本机数据库。',
  '删除会话后应用内不可恢复，不承诺物理擦除。',
  '任务审计记录会保留，且不含正文。',
  '应用被强制结束时，本次未提交的生成内容可能丢失。',
  '上下文可能省略较早的完整轮次，被省略的轮次不会发送给模型。',
  '带工具的轮次不进入后续上下文（工具往返不保留）。',
];

export default function Disclosures() {
  return (
    <details open className="rounded-md border border-border bg-muted px-3 py-2 text-xs text-muted-foreground">
      <summary className={cn('cursor-pointer font-medium text-foreground', FOCUS)}>数据与隐私说明</summary>
      <ul className="m-0 mt-2 list-disc pl-5">
        {DISCLOSURES.map(item => <li key={item} className="my-0.5">{item}</li>)}
      </ul>
    </details>
  );
}
