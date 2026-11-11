import { useId, useMemo, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { useLocale } from '../../i18n';
import type { Json } from '../../stores/agentRunnerStore';
import { elicitationSizeValid, elicitationValueIssue, inspectElicitation, type ElicitationFields, type ElicitationField } from './elicitationSchema';
import './agent-native-request.css';

export function elicitationDefaults(schema: ElicitationFields): ElicitationFields {
  const form = inspectElicitation(schema);
  if (!form) return {};
  const values = Object.fromEntries(form.fields.flatMap(field => {
    const value = field.schema.default;
    return field.supported && value !== undefined && !elicitationValueIssue(field, value) ? [[field.key, value]] : [];
  }));
  return elicitationSizeValid(values) ? values : {};
}

export function elicitationValid(schema: ElicitationFields, values: ElicitationFields): boolean {
  const form = inspectElicitation(schema);
  return !!form && elicitationSizeValid({ action: 'accept', content: values })
    && Object.keys(values).every(key => Object.prototype.hasOwnProperty.call(form.properties, key))
    && form.fields.every(field => !elicitationValueIssue(field, Object.prototype.hasOwnProperty.call(values, field.key) ? values[field.key] : undefined));
}

function ElicitationControl({ field, value, update }: { field: ElicitationField; value: Json | undefined; update: (value: Json | undefined) => void }) {
  const { t } = useLocale();
  const id = useId();
  const [numberDraft, setNumberDraft] = useState<{ raw: string; value: Json | undefined }>();
  const { schema, choices, key, supported, required } = field;
  const label = typeof schema.title === 'string' && schema.title ? schema.title : key;
  const description = typeof schema.description === 'string' ? schema.description : '';
  const constraints = (['minimum', 'maximum', 'minLength', 'maxLength', 'minItems', 'maxItems'] as const).flatMap(key => typeof schema[key] === 'number' ? [t(`agentRequest.constraint.${key}`, { bound: String(schema[key]) })] : []);
  if (schema.type === 'integer') constraints.unshift(t('agentRequest.constraint.integer'));
  if (typeof schema.format === 'string') constraints.push(t(`agentRequest.constraint.${schema.format}`));
  const hintText = [description, ...constraints].filter(Boolean).join(' ');
  const hintId = hintText ? `${id}-hint` : undefined;
  const labelText = `${label}${required ? ` (${t('agent.requiredAnswer')})` : ''}`;
  if (!supported) return <div className="agent-native-request-group agent-native-request-field">
    <strong>{labelText}</strong>
    {description ? <p className="agent-native-request-note">{description}</p> : null}
    <p role={required ? 'alert' : undefined} className={required ? 'agent-native-request-error' : 'agent-native-request-note'}>{t(required ? 'agentRequest.fieldUnsupportedRequired' : 'agentRequest.fieldUnsupportedOptional')}</p>
  </div>;

  const issue = elicitationValueIssue(field, value);
  const error = issue ? t(`agentRequest.validation.${issue.code}`, issue.bound === undefined ? undefined : { bound: String(issue.bound) }) : undefined;
  const aria = {
    'aria-invalid': error ? true as const : undefined,
    'aria-describedby': [hintId, error && `${id}-error`].filter(Boolean).join(' ') || undefined,
  };
  const hint = hintId ? <p className="agent-native-request-note" id={hintId}>{hintText}</p> : null;
  if (schema.type === 'boolean' || choices) {
    const array = schema.type === 'array';
    const checked = Array.isArray(value) ? value : [];
    const canUseEmpty = array && (!schema.minItems || schema.minItems === 0);
    const options = schema.type === 'boolean'
      ? [{ value: true, label: t('agent.yes') }, { value: false, label: t('agent.no') }]
      : (choices ?? []).map(choice => ({ ...choice, label: choice.label || t('agentRequest.emptyString') }));
    return <fieldset className="agent-native-request-group agent-native-request-field" {...aria}>
      <legend>{labelText}</legend>
      {hint}
      <div className="agent-native-request-choices">
      {!required && !array ? <label className="agent-native-request-option" data-selected={value === undefined}><input type="radio" name={id} checked={value === undefined} onChange={() => update(undefined)} {...aria} /><span>{t('agent.unanswered')}</span></label> : null}
      {options.map(option => <label className="agent-native-request-option" data-selected={array ? checked.includes(option.value) : value === option.value} key={String(option.value)}>
        <input type={array ? 'checkbox' : 'radio'} name={id} value={String(option.value)} checked={array ? checked.includes(option.value) : value === option.value}
          aria-required={!array && required || undefined} onChange={event => update(array ? event.target.checked ? [...checked, option.value] : checked.filter(item => item !== option.value) : option.value)} {...aria} />
        <span>{option.label}</span>
      </label>)}
      </div>
      {array ? <div className="agent-native-request-actions">
        {canUseEmpty && value === undefined ? <Button size="xs" variant="secondary" onClick={() => update([])}>{t('agentRequest.useEmptyList')}</Button> : null}
        {!required && value !== undefined ? <Button size="xs" variant="secondary" onClick={() => update(undefined)}>{t('agentRequest.clearOptional')}</Button> : null}
      </div> : null}
      {error ? <p className="agent-native-request-error" id={`${id}-error`}>{error}</p> : null}
    </fieldset>;
  }

  const numeric = schema.type === 'number' || schema.type === 'integer';
  const canUseEmpty = schema.type === 'string' && !schema.minLength && schema.format == null;
  return <div className="agent-native-request-group agent-native-request-field">
    <Input id={id} label={labelText} aria-describedby={hintId} aria-required={required || undefined} error={error} autoComplete="off"
      type={schema.format === 'email' ? 'email' : 'text'} inputMode={numeric ? 'decimal' : undefined}
      value={numeric && numberDraft && Object.is(numberDraft.value, value) ? numberDraft.raw : value === undefined ? '' : String(value)}
      onChange={event => {
        const raw = event.target.value;
        if (!numeric) { update(raw); return; }
        // Keep unfinished input only while it still represents the controlled value.
        const number = /^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(raw) ? Number(raw) : NaN;
        const next = raw === '' ? undefined : Number.isFinite(number) ? number : raw;
        setNumberDraft({ raw, value: next });
        update(next);
      }} />
    {hint}
    <div className="agent-native-request-actions">
      {canUseEmpty && value === undefined ? <Button size="xs" variant="secondary" onClick={() => update('')}>{t('agentRequest.useEmptyText')}</Button> : null}
      {!required && value !== undefined ? <Button size="xs" variant="secondary" onClick={() => { setNumberDraft(undefined); update(undefined); }}>{t('agentRequest.clearOptional')}</Button> : null}
    </div>
  </div>;
}

export function NativeElicitation({ schema, values, disabled, onChange }: { schema: ElicitationFields; values: ElicitationFields; disabled: boolean; onChange: (values: ElicitationFields) => void }) {
  const { t } = useLocale();
  const form = useMemo(() => inspectElicitation(schema), [schema]);
  if (!form) return <p role="alert" className="agent-native-request-error">{t('agentRequest.formUnsupported')}</p>;
  const update = (key: string, value: Json | undefined) => {
    const next: ElicitationFields = value === undefined ? { ...values } : { ...values, [key]: value };
    if (value === undefined) delete next[key];
    onChange(next);
  };
  return <fieldset disabled={disabled} className="agent-native-request-group agent-native-request-form">
    <legend>{typeof schema.title === 'string' && schema.title || t('agentRequest.formFields')}</legend>
    {typeof schema.description === 'string' && schema.description ? <p className="agent-native-request-note">{schema.description}</p> : null}
    {form.fields.map(field => <ElicitationControl key={field.key} field={field} value={Object.prototype.hasOwnProperty.call(values, field.key) ? values[field.key] : undefined} update={value => update(field.key, value)} />)}
    {!elicitationSizeValid({ action: 'accept', content: values }) ? <p role="alert" className="agent-native-request-error">{t('agentRequest.formTooLarge')}</p> : null}
  </fieldset>;
}
