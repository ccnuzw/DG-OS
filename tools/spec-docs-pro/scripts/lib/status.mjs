// Single source of truth for status vocabularies. Mirrors references/status-and-ids.md.

export const IMPLEMENTATION_STATUS = [
  { key: "planned", label: "规划中", aliases: ["规划中", "规划", "设计中", "未开始", "planned", "todo", "not started"] },
  { key: "in-progress", label: "开发中", aliases: ["开发中", "进行中", "基础实现", "部分实现", "实现中", "implementing", "in progress", "partial", "foundation"] },
  { key: "local-verified", label: "本地验证", aliases: ["本地验证", "本地通过", "local verified", "local-verified"] },
  { key: "pending-acceptance", label: "待验收", aliases: ["待验收", "待确认", "pending acceptance", "ready for acceptance"] },
  { key: "done", label: "已完成", aliases: ["已完成", "完成", "已上线", "已发布", "done", "completed", "production verified"] },
  { key: "blocked", label: "阻塞", aliases: ["阻塞", "受阻", "blocked"] },
  { key: "deprecated", label: "已废弃", aliases: ["已废弃", "废弃", "已下线", "deprecated", "retired"] }
];

export const SPEC_STATUS = [
  { key: "draft", label: "Draft", aliases: ["draft", "草稿", "起草中"] },
  { key: "ready", label: "Ready", aliases: ["ready", "就绪", "可开发", "已冻结"] },
  { key: "deprecated", label: "Deprecated", aliases: ["deprecated", "已废弃", "废弃"] }
];

export const EVIDENCE_LEVEL = [
  { key: "none", label: "未创建", aliases: ["未创建", "目标文件未创建", "规划中", "planned"] },
  { key: "created", label: "已创建", aliases: ["已创建", "待补充", "目标文件已存在"] },
  { key: "static", label: "静态核验", aliases: ["静态核验", "静态审计", "文档核验"] },
  { key: "local", label: "本地通过", aliases: ["本地通过", "已通过", "通过", "后端通过", "前端通过", "local verified"] },
  { key: "staging", label: "预发通过", aliases: ["预发通过", "集成环境通过", "staging"] },
  { key: "production", label: "生产通过", aliases: ["生产通过", "生产验证", "production verified"] },
  { key: "failed", label: "失败", aliases: ["失败", "未通过", "failed"] },
  { key: "na", label: "不适用", aliases: ["不适用", "n/a", "na"] }
];

export const DECISION_STATUS = [
  { key: "proposed", label: "草案", aliases: ["草案", "待定", "设计阶段", "proposed"] },
  { key: "frozen", label: "已冻结", aliases: ["已冻结", "冻结", "accepted"] },
  { key: "superseded", label: "已取代", aliases: ["已取代", "已被", "取代", "superseded"] },
  { key: "revoked", label: "已撤销", aliases: ["已撤销", "撤销", "deprecated"] }
];

// Exact match first, then prefix, then substring for short cells only.
// Substring matching on long prose is unreliable ("未完成" must not read as "已完成").
function matcher(table) {
  const entries = table.flatMap(item => item.aliases.map(alias => ({ alias: alias.toLowerCase(), item })));
  entries.sort((a, b) => b.alias.length - a.alias.length);
  return value => {
    const text = String(value || "").trim().toLowerCase();
    if (!text) return null;
    const exact = entries.find(entry => entry.alias === text || entry.item.key === text);
    if (exact) return exact.item;
    const negated = alias => text.includes(`未${alias}`) || text.includes(`不${alias}`) || text.includes(`not ${alias}`);
    const prefix = entries.find(entry => text.startsWith(entry.alias) && !negated(entry.alias));
    if (prefix) return prefix.item;
    if (text.length > 14) return null;
    const hit = entries.find(entry => text.includes(entry.alias) && !negated(entry.alias));
    return hit ? hit.item : null;
  };
}

export const matchImplementationStatus = matcher(IMPLEMENTATION_STATUS);
export const matchSpecStatus = matcher(SPEC_STATUS);
export const matchEvidenceLevel = matcher(EVIDENCE_LEVEL);
export const matchDecisionStatus = matcher(DECISION_STATUS);

export function isCanonical(table, value) {
  return table.some(item => item.label === String(value || "").trim());
}
