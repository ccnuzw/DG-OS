export type MessageVariables = Readonly<Record<string, string | number>>;

/** Missing resources never fall back to another language (FR-016). */
export function formatMessage(catalog: Readonly<Record<string, string>>, key: string, variables?: MessageVariables): string {
  const template=catalog[key];
  if(typeof template!=='string')return `⟦${key}⟧`;
  const parameters=[...template.matchAll(/\{([a-zA-Z0-9_]+)\}/g)].map(match=>match[1]);
  if(parameters.some(name=>!variables||!Object.prototype.hasOwnProperty.call(variables,name)))return `⟦${key}⟧`;
  return template.replace(/\{([a-zA-Z0-9_]+)\}/g,(_match,name:string)=>String(variables![name]));
}
