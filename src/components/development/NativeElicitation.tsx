import { useId } from 'react';
import { Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Json } from '../../stores/agentRunnerStore';

type Fields = Record<string, Json>;
function object(value: Json | undefined): Fields { return value !== null && typeof value === 'object' && !Array.isArray(value) ? value : {}; }
function options(schema: Fields): { value: string; label: string }[] | null {
  if (Array.isArray(schema.enum)) return schema.enum.filter((value): value is string => typeof value === 'string').map((value, index) => ({ value, label: Array.isArray(schema.enumNames) && typeof schema.enumNames[index] === 'string' ? schema.enumNames[index] as string : value }));
  const titled = schema.oneOf ?? schema.anyOf;
  if (Array.isArray(titled)) return titled.flatMap(value => { const row = object(value); return typeof row.const === 'string' ? [{ value: row.const, label: typeof row.title === 'string' ? row.title : row.const }] : []; });
  return null;
}
function supported(field: Fields): boolean {
  return ['string', 'number', 'integer', 'boolean'].includes(String(field.type)) || field.type === 'array' && options(object(field.items)) !== null;
}
export function elicitationDefaults(schema: Fields): Fields {
  const entries = Object.entries(object(schema.properties));
  return Object.fromEntries(entries.flatMap(([key, value]) => { const field = object(value); return supported(field) && field.default !== undefined && field.default !== null ? [[key, field.default]] : []; }));
}
function validField(field: Fields, value: Json): boolean {
  if (!supported(field)) return false;
  if (field.type === 'boolean') return typeof value === 'boolean';
  if (field.type === 'number' || field.type === 'integer') return typeof value === 'number' && Number.isFinite(value) && (field.type !== 'integer' || Number.isInteger(value))
    && (typeof field.minimum !== 'number' || value >= field.minimum) && (typeof field.maximum !== 'number' || value <= field.maximum);
  if (field.type === 'array') {
    const choices = options(object(field.items)) ?? [];
    return Array.isArray(value) && value.every(item => typeof item === 'string' && choices.some(choice => choice.value === item)) && new Set(value).size === value.length
      && (typeof field.minItems !== 'number' || value.length >= field.minItems) && (typeof field.maxItems !== 'number' || value.length <= field.maxItems);
  }
  if (typeof value !== 'string') return false;
  const choices = options(field);
  if (choices && !choices.some(choice => choice.value === value)) return false;
  const length = [...value].length;
  if (typeof field.minLength === 'number' && length < field.minLength || typeof field.maxLength === 'number' && length > field.maxLength) return false;
  if (field.format === 'email' && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value)) return false;
  if (field.format === 'uri') { try { new URL(value); } catch { return false; } }
  if (field.format === 'date' && (!/^\d{4}-\d{2}-\d{2}$/.test(value) || !Number.isFinite(Date.parse(value)) || new Date(value).toISOString().slice(0, 10) !== value)) return false;
  if (field.format === 'date-time' && (!/^\d{4}-\d{2}-\d{2}T/.test(value) || !Number.isFinite(Date.parse(value)))) return false;
  return true;
}
export function elicitationValid(schema: Fields, values: Fields): boolean {
  if (schema.type !== 'object') return false;
  const fields = object(schema.properties);
  if (Object.values(fields).some(field => !supported(object(field)))) return false;
  if (Array.isArray(schema.required) && schema.required.some(key => typeof key !== 'string' || values[key] === undefined)) return false;
  return Object.entries(values).every(([key, value]) => key in fields && validField(object(fields[key]), value));
}
export function NativeElicitation({ schema, values, disabled, onChange }: { schema: Fields; values: Fields; disabled: boolean; onChange: (values: Fields) => void }) {
  const { t } = useLocale();
  const formId = useId();
  const required = Array.isArray(schema.required) ? schema.required : [];
  const update = (key: string, value: Json | undefined) => { const next = { ...values }; if (value === undefined) delete next[key]; else next[key] = value; onChange(next); };
  return <fieldset disabled={disabled} className="grid min-w-0 gap-3">{Object.entries(object(schema.properties)).map(([key, raw]) => {
    const field = object(raw); const label = typeof field.title === 'string' ? field.title : key;
    const choices = options(field.type === 'array' ? object(field.items) : field);
    const value = values[key];
    const hint = typeof field.description === 'string' ? field.description : undefined;
    if (!supported(field)) return <p key={key} role="alert">{label}: {t('agentExecution.unsupported')}</p>;
    if (field.type === 'boolean' || choices) return <fieldset key={key} className="grid min-w-0 gap-2"><legend>{label}{required.includes(key) ? ` (${t('agent.requiredAnswer')})` : ''}</legend>{hint ? <p className="m-0 text-sm">{hint}</p> : null}{field.type === 'boolean' ? [undefined, true, false].map(option => <label key={String(option)} className="flex items-start gap-2"><input type="radio" name={`${formId}-${key}`} checked={value === option} onChange={() => update(key, option)} />{t(option === undefined ? 'agent.unanswered' : option ? 'agent.yes' : 'agent.no')}</label>) : choices?.map(choice => <label className="flex items-start gap-2" key={choice.value}><input type={field.type === 'array' ? 'checkbox' : 'radio'} name={`${formId}-${key}`} value={choice.value} checked={field.type === 'array' ? Array.isArray(value) && value.includes(choice.value) : value === choice.value} onChange={event => update(key, field.type === 'array' ? event.target.checked ? [...(Array.isArray(value) ? value : []), choice.value] : (Array.isArray(value) ? value : []).filter(item => item !== choice.value) : choice.value)} />{choice.label}</label>)}</fieldset>;
    const numeric = field.type === 'number' || field.type === 'integer';
    return <Input key={key} label={`${label}${required.includes(key) ? ` (${t('agent.requiredAnswer')})` : ''}`} title={hint} type={numeric ? 'number' : field.format === 'email' ? 'email' : field.format === 'date' ? 'date' : 'text'} step={field.type === 'integer' ? 1 : 'any'} value={value === undefined ? '' : String(value)} onChange={event => update(key, numeric ? event.target.value === '' ? undefined : Number(event.target.value) : event.target.value)} />;
  })}{!elicitationValid(schema, values) ? <p role="status" className="m-0 text-sm">{t('agent.completeForm')}</p> : null}</fieldset>;
}
