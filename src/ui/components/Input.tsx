import { cn } from '../lib/cn';
import { accessibleName, omitStyle, type DomProps, type Named } from '../lib/props';
import { CONTROL_SURFACE, CONTROL_SURFACE_BASE, Field, controlBorder, useFieldIds } from './Field';

interface FieldExtras {
  /** Error text shown under the control. Sets `aria-invalid` and is linked through `aria-describedby`. */
  error?: string;
  /** Green border for a validated value. Ignored while `error` is set. */
  success?: boolean;
}

// FR-005 controls plus the semantic search field used by V2 page patterns.
export type InputType = 'text' | 'password' | 'email' | 'number' | 'search';

export type SingleLineInputProps = Omit<DomProps<'input'>, 'type'> &
  FieldExtras &
  Named & { multiline?: false; type?: InputType };

/** AC27: a textarea with the same label/error behaviour. Uses UI typography unless className selects type-reading/type-code. */
export type MultilineInputProps = DomProps<'textarea'> & FieldExtras & Named & { multiline: true; rows?: number };

export type InputProps = SingleLineInputProps | MultilineInputProps;

// Keep UI defaults out of explicit content roles: Tailwind utility order otherwise overrides their size/leading.
function inputSurface(className: string | undefined): string {
  return /(?:^|\s)(?:type-reading|type-code|font-mono)(?=\s|$)/.test(className ?? '')
    ? CONTROL_SURFACE_BASE
    : CONTROL_SURFACE;
}

export function Input(props: InputProps) {
  return props.multiline ? <TextareaField {...props} /> : <InputField {...props} />;
}

function InputField(props: SingleLineInputProps) {
  const {
    label,
    error,
    success = false,
    id,
    className,
    type = 'text',
    multiline: _multiline,
    'aria-label': ariaLabel,
    'aria-labelledby': ariaLabelledby,
    ...rest
  } = omitStyle(props);
  const ids = useFieldIds(id, error, rest['aria-describedby']);
  return (
    <Field id={ids.fieldId} label={label} error={error} errorId={ids.errorId}>
      <input
        {...rest}
        {...accessibleName({ label, 'aria-label': ariaLabel, 'aria-labelledby': ariaLabelledby })}
        id={ids.fieldId}
        type={type}
        aria-invalid={error ? true : undefined}
        aria-describedby={ids.describedBy}
        className={cn(inputSurface(className), 'h-(--control-height-compact)', controlBorder(error, success), className)}
      />
    </Field>
  );
}

function TextareaField(props: MultilineInputProps) {
  const {
    label,
    error,
    success = false,
    id,
    className,
    rows = 3,
    multiline: _multiline,
    'aria-label': ariaLabel,
    'aria-labelledby': ariaLabelledby,
    ...rest
  } = omitStyle(props);
  const ids = useFieldIds(id, error, rest['aria-describedby']);
  return (
    <Field id={ids.fieldId} label={label} error={error} errorId={ids.errorId}>
      <textarea
        {...rest}
        {...accessibleName({ label, 'aria-label': ariaLabel, 'aria-labelledby': ariaLabelledby })}
        id={ids.fieldId}
        rows={rows}
        aria-invalid={error ? true : undefined}
        aria-describedby={ids.describedBy}
        className={cn(inputSurface(className), 'min-h-16 py-2', controlBorder(error, success), className)}
      />
    </Field>
  );
}
