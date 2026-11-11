import type { Json } from '../../stores/agentRunnerStore';

export type ElicitationFields = Record<string, Json>;
type Choice = { value: string; label: string };
export interface ElicitationField {
  key: string;
  schema: ElicitationFields;
  required: boolean;
  supported: boolean;
  choices?: Choice[];
}

const rootKeys = ['type', 'properties', 'required', 'additionalProperties', 'title', 'description', '$schema'];
const annotations = ['title', 'description', 'default'];
const choiceKeys = ['enum', 'enumNames', 'oneOf', 'anyOf'];
const formats = ['email', 'uri', 'date', 'date-time'];
const isObject = (value: Json | undefined): value is ElicitationFields => value != null && typeof value === 'object' && !Array.isArray(value);
const keysAllowed = (value: ElicitationFields, keys: string[]) => Object.keys(value).every(key => keys.includes(key));
const isString = (value: Json): value is string => typeof value === 'string';
const isCount = (value: Json): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
const isNumber = (value: Json): value is number => typeof value === 'number' && Number.isFinite(value);

export function elicitationSizeValid(value: ElicitationFields) {
  const encoder = new TextEncoder();
  function size(value: Json, depth: number): number {
    if (depth > 16) return Infinity;
    if (typeof value === 'string') return encoder.encode(value).length * 6 + 2;
    if (Array.isArray(value)) {
      let bytes = 2;
      for (const entry of value) { bytes += size(entry, depth + 1) + 1; if (bytes > 65536) return bytes; }
      return bytes;
    }
    if (isObject(value)) {
      let bytes = 2;
      for (const [key, entry] of Object.entries(value)) { bytes += encoder.encode(key).length * 6 + size(entry, depth + 1) + 4; if (bytes > 65536) return bytes; }
      return bytes;
    }
    return 32;
  }
  return size(value, 0) <= 65536;
}

function orderedBounds(min: Json | undefined, max: Json | undefined, valid: (value: Json) => value is number) {
  return (min == null || valid(min)) && (max == null || valid(max)) && (min == null || max == null || min <= max);
}

/** Undefined means no enum, null means malformed. Values are never split on punctuation. */
function readChoices(schema: ElicitationFields): Choice[] | null | undefined {
  const offered = ['enum', 'oneOf', 'anyOf'].filter(key => schema[key] != null);
  if (!offered.length) return choiceKeys.some(key => Object.prototype.hasOwnProperty.call(schema, key)) ? null : undefined;
  if (offered.length !== 1) return null;
  const entries = schema[offered[0]];
  if (!Array.isArray(entries) || !entries.length || entries.length > 256) return null;
  let choices: Choice[];
  if (offered[0] === 'enum') {
    const labels = schema.enumNames;
    if (!entries.every(isString) || (labels != null && (!Array.isArray(labels) || labels.length !== entries.length || !labels.every(isString)))) return null;
    choices = entries.map((value, index) => {
      const label = Array.isArray(labels) ? labels[index] : undefined;
      return { value, label: typeof label === 'string' ? label : value };
    });
  } else {
    if (schema.enumNames != null) return null;
    choices = [];
    for (const option of entries) {
      if (!isObject(option) || !keysAllowed(option, ['const', 'title']) || typeof option.const !== 'string' || (option.title !== undefined && typeof option.title !== 'string')) return null;
      choices.push({ value: option.const, label: typeof option.title === 'string' ? option.title : option.const });
    }
  }
  return new Set(choices.map(choice => choice.value)).size === choices.length ? choices : null;
}

function fieldSupport(schema: ElicitationFields): { choices?: Choice[] } | null {
  if (![schema.title, schema.description].every(value => value == null || typeof value === 'string')) return null;
  const keys = ['type', ...annotations];
  switch (schema.type) {
    case 'boolean':
      return keysAllowed(schema, keys) ? {} : null;
    case 'number':
    case 'integer':
      return keysAllowed(schema, [...keys, 'minimum', 'maximum']) && orderedBounds(schema.minimum, schema.maximum, isNumber) ? {} : null;
    case 'string': {
      if (!keysAllowed(schema, [...keys, ...choiceKeys, 'minLength', 'maxLength', 'format']) || !orderedBounds(schema.minLength, schema.maxLength, isCount)
        || (schema.format != null && (typeof schema.format !== 'string' || !formats.includes(schema.format)))) return null;
      const choices = readChoices(schema);
      return choices === null ? null : { choices };
    }
    case 'array': {
      const items = schema.items;
      if (!keysAllowed(schema, [...keys, 'items', 'minItems', 'maxItems', 'uniqueItems']) || !orderedBounds(schema.minItems, schema.maxItems, isCount)
        || (schema.uniqueItems !== undefined && typeof schema.uniqueItems !== 'boolean') || !isObject(items)
        || (items.type !== undefined && items.type !== 'string') || !keysAllowed(items, ['type', ...choiceKeys])) return null;
      const choices = readChoices(items);
      return choices ? { choices } : null;
    }
    default:
      return null;
  }
}

/** Keep in step with dgos-agents' Codex elicitation validator. Unhandled constraints never silently validate. */
export function inspectElicitation(schema: ElicitationFields): { properties: ElicitationFields; fields: ElicitationField[] } | null {
  const properties = schema.properties;
  const required = schema.required;
  if (schema.type !== 'object' || !keysAllowed(schema, rootKeys) || ![schema.title, schema.description].every(value => value == null || typeof value === 'string') || !isObject(properties)
    || Object.keys(properties).length > 64 || !elicitationSizeValid(schema)
    || (schema.$schema != null && typeof schema.$schema !== 'string')
    || (schema.additionalProperties !== undefined && typeof schema.additionalProperties !== 'boolean')
    || (required != null && (!Array.isArray(required) || !required.every(key => typeof key === 'string' && Object.prototype.hasOwnProperty.call(properties, key)) || new Set(required).size !== required.length))) return null;
  const requiredKeys = new Set(Array.isArray(required) ? required : []);
  return { properties, fields: Object.entries(properties).map(([key, value]) => {
    const schema = isObject(value) ? value : {};
    const support = fieldSupport(schema);
    return { key, schema, required: requiredKeys.has(key), supported: support !== null, choices: support?.choices };
  }) };
}

type IssueCode = 'required' | 'unsupported' | 'boolean' | 'number' | 'integer' | 'minimum' | 'maximum' | 'choice' | 'unique' | 'minItems' | 'maxItems' | 'text' | 'minLength' | 'maxLength' | 'email' | 'uri' | 'date' | 'dateTime';
type Issue = { code: IssueCode; bound?: number };

function matchesDate(value: string) {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!parts) return false;
  const [year, month, day] = parts.slice(1).map(Number);
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  return month >= 1 && month <= 12 && day >= 1 && day <= [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
}

function matchesDateTime(value: string) {
  const parts = /^(\d{4}-\d{2}-\d{2})[Tt]([01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:[Zz]|[+-](?:[01]\d|2[0-3]):[0-5]\d)$/.exec(value);
  return !!parts && matchesDate(parts[1]);
}

export function elicitationValueIssue(field: ElicitationField, value: Json | undefined): Issue | undefined {
  if (!field.supported) return field.required || value !== undefined ? { code: 'unsupported' } : undefined;
  if (value === undefined) return field.required ? { code: 'required' } : undefined;
  const schema = field.schema;
  if (schema.type === 'boolean') return typeof value === 'boolean' ? undefined : { code: 'boolean' };
  if (schema.type === 'number' || schema.type === 'integer') {
    if (!isNumber(value)) return { code: 'number' };
    if (schema.type === 'integer' && !Number.isInteger(value)) return { code: 'integer' };
    if (typeof schema.minimum === 'number' && value < schema.minimum) return { code: 'minimum', bound: schema.minimum };
    if (typeof schema.maximum === 'number' && value > schema.maximum) return { code: 'maximum', bound: schema.maximum };
    return undefined;
  }
  if (schema.type === 'array') {
    const choices = new Set(field.choices?.map(choice => choice.value));
    if (!Array.isArray(value) || !value.every(item => typeof item === 'string' && choices.has(item))) return { code: 'choice' };
    if (new Set(value).size !== value.length) return { code: 'unique' };
    if (typeof schema.minItems === 'number' && value.length < schema.minItems) return { code: 'minItems', bound: schema.minItems };
    if (typeof schema.maxItems === 'number' && value.length > schema.maxItems) return { code: 'maxItems', bound: schema.maxItems };
    return undefined;
  }
  if (typeof value !== 'string') return { code: 'text' };
  if (field.choices && !field.choices.some(choice => choice.value === value)) return { code: 'choice' };
  const length = [...value].length;
  if (typeof schema.minLength === 'number' && length < schema.minLength) return { code: 'minLength', bound: schema.minLength };
  if (typeof schema.maxLength === 'number' && length > schema.maxLength) return { code: 'maxLength', bound: schema.maxLength };
  if (schema.format === 'email') {
    const at = value.indexOf('@');
    const domain = value.slice(at + 1);
    const dot = domain.indexOf('.');
    if (at <= 0 || domain.includes('@') || dot <= 0 || dot === domain.length - 1 || /\s/u.test(value)) return { code: 'email' };
  }
  if (schema.format === 'uri') { try { new URL(value); } catch { return { code: 'uri' }; } }
  if (schema.format === 'date' && !matchesDate(value)) return { code: 'date' };
  if (schema.format === 'date-time' && !matchesDateTime(value)) return { code: 'dateTime' };
  return undefined;
}
