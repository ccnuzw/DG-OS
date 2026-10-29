// AC05, AC06, AC13, AC14, AC24, AC26, AC27 (FR-005 Phase 4): controls and display components of `@dgos/ui`.
//
// What this file can and cannot prove (jsdom, no layout, no CSS variable resolution):
//  - Proven here: roles, accessible names, label/aria wiring, controlled values, disabled/loading behaviour,
//    Tabs/Radio arrow keys, the Stack gap class table, `style` being dropped, and that every interactive
//    component carries a focus-visible utility which the real Tailwind compiler turns into an outline that
//    reads --color-focus-ring.
//  - NOT proven here, left to a real WebView (Phase 8): that the focus ring is actually visible and not clipped,
//    real focus movement after Tab/arrow keys in WKWebView, Select popper placement and the trigger-width
//    variable, spinner animation, and Space activating a native button. jsdom does not turn a `keydown` Space
//    into a click, so Space is tested by the click that a browser dispatches for it (marked below), not by key
//    synthesis; Radix Checkbox and Switch are native `<button>`s and rely on that browser behaviour.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import {
  Button,
  Card,
  Checkbox,
  Divider,
  Input,
  Loading,
  RadioGroup,
  Select,
  Stack,
  Switch,
  Tabs,
} from '@dgos/ui';
import { FOCUS_RING, FOCUS_RING_INSET } from '../src/ui/lib/focus';
import { compileUi, contrast, themeTokens } from './helpers/designTokens';

afterEach(cleanup);

/** Tests hand components props the types forbid, the way untyped module code (D067) could. */
const untyped = <T,>(props: object) => props as unknown as T;

describe('Button (AC05)', () => {
  const variants = ['primary', 'secondary', 'danger', 'ghost', 'link'] as const;
  const sizes = ['sm', 'md', 'lg'] as const;

  it('renders each variant with its own token classes', () => {
    const expected: Record<(typeof variants)[number], string[]> = {
      primary: ['bg-primary', 'text-primary-foreground'],
      secondary: ['bg-secondary', 'border-input'],
      danger: ['bg-destructive', 'text-destructive-foreground'],
      ghost: ['bg-transparent'],
      link: ['text-primary', 'px-0', 'h-auto'],
    };
    for (const variant of variants) {
      const { unmount } = render(<Button variant={variant}>{variant}</Button>);
      const classes = screen.getByRole('button', { name: variant }).className.split(/\s+/);
      for (const name of expected[variant]) expect(classes, `${variant} -> ${name}`).toContain(name);
      unmount();
    }
  });

  it('maps sizes to the 32/36/40px height tokens', () => {
    const height = { sm: 'h-8', md: 'h-9', lg: 'h-10' };
    for (const size of sizes) {
      const { unmount } = render(<Button size={size}>go</Button>);
      expect(screen.getByRole('button').className.split(/\s+/)).toContain(height[size]);
      unmount();
    }
  });

  it('defaults to primary, md and type="button" (never an accidental form submit)', () => {
    render(<Button>save</Button>);
    const button = screen.getByRole('button', { name: 'save' });
    expect(button.getAttribute('type')).toBe('button');
    expect(button.className).toContain('bg-primary');
    expect(button.className).toContain('h-9');
  });

  it('calls onClick when enabled', () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>go</Button>);
    fireEvent.click(screen.getByRole('button'));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('uses the native disabled attribute and does not call onClick', () => {
    const onClick = vi.fn();
    render(<Button disabled onClick={onClick}>go</Button>);
    const button = screen.getByRole('button') as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('loading sets aria-busy, shows a hidden spinner, stays focusable and swallows clicks', () => {
    const onClick = vi.fn();
    render(<Button loading onClick={onClick}>deleting</Button>);
    const button = screen.getByRole('button', { name: 'deleting' }) as HTMLButtonElement;
    expect(button.getAttribute('aria-busy')).toBe('true');
    expect(button.disabled).toBe(false);
    expect(button.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    button.focus();
    expect(document.activeElement).toBe(button);
    fireEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it('loading stops a submit button from submitting the form', () => {
    const onSubmit = vi.fn((event: { preventDefault: () => void }) => event.preventDefault());
    render(
      <form onSubmit={onSubmit}>
        <Button type="submit" loading>send</Button>
      </form>,
    );
    fireEvent.click(screen.getByRole('button', { name: 'send' }));
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('has no aria-busy when idle, and no forced aria-label with visible text', () => {
    render(<Button>save</Button>);
    const button = screen.getByRole('button', { name: 'save' });
    expect(button.hasAttribute('aria-busy')).toBe(false);
    expect(button.hasAttribute('aria-label')).toBe(false);
  });

  it('icon-only form takes its name from aria-label and drops the glyph while loading', () => {
    const { rerender } = render(
      <Button iconOnly aria-label="close"><svg data-testid="glyph" /></Button>,
    );
    expect(screen.getByRole('button', { name: 'close' })).toBeTruthy();
    expect(screen.getByTestId('glyph')).toBeTruthy();
    rerender(<Button iconOnly aria-label="close" loading><svg data-testid="glyph" /></Button>);
    expect(screen.queryByTestId('glyph')).toBeNull();
    expect(screen.getByRole('button', { name: 'close' }).getAttribute('aria-busy')).toBe('true');
  });

  it('an icon-only button without aria-label is a type error', () => {
    // Compile-time contract: `tsc` does not cover tests/, so this only documents it. The directive below is
    // checked by `npx tsc --noEmit -p tsconfig.tests.json`-style runs; at runtime nothing is asserted about it.
    // @ts-expect-error iconOnly requires aria-label
    const element = <Button iconOnly><svg /></Button>;
    expect(element).toBeTruthy();
  });
});

describe('Input (AC06)', () => {
  it.each(['text', 'password', 'email', 'number'] as const)('type=%s: label is associated with the control', (type) => {
    render(<Input type={type} label="Name" />);
    const control = screen.getByLabelText('Name') as HTMLInputElement;
    expect(control.tagName).toBe('INPUT');
    expect(control.type).toBe(type);
  });

  it('defaults to type=text', () => {
    render(<Input label="Name" />);
    expect((screen.getByLabelText('Name') as HTMLInputElement).type).toBe('text');
  });

  it('error sets aria-invalid, renders the text after the control and links it with aria-describedby', () => {
    render(<Input label="Name" error="Name already exists" />);
    const control = screen.getByLabelText('Name');
    expect(control.getAttribute('aria-invalid')).toBe('true');
    const message = screen.getByText('Name already exists');
    expect(control.getAttribute('aria-describedby')).toBe(message.id);
    expect(control.compareDocumentPosition(message) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('has neither aria-invalid nor aria-describedby without an error', () => {
    render(<Input label="Name" />);
    const control = screen.getByLabelText('Name');
    expect(control.hasAttribute('aria-invalid')).toBe(false);
    expect(control.hasAttribute('aria-describedby')).toBe(false);
  });

  it('keeps a caller aria-describedby next to the error id', () => {
    render(<><p id="hint">At most 20 characters</p><Input label="Name" aria-describedby="hint" error="Too long" /></>);
    const ids = (screen.getByLabelText('Name').getAttribute('aria-describedby') ?? '').split(' ');
    expect(ids).toContain('hint');
    expect(ids).toContain(screen.getByText('Too long').id);
  });

  it('error border wins over success', () => {
    const { rerender } = render(<Input label="Name" success />);
    expect(screen.getByLabelText('Name').className).toContain('border-success');
    rerender(<Input label="Name" success error="bad" />);
    const classes = screen.getByLabelText('Name').className.split(/\s+/);
    expect(classes).toContain('border-error');
    expect(classes).not.toContain('border-success');
  });

  it('disabled is not editable', () => {
    const onChange = vi.fn();
    render(<Input label="Name" disabled value="fixed" onChange={onChange} />);
    const control = screen.getByLabelText('Name') as HTMLInputElement;
    expect(control.disabled).toBe(true);
    expect(control.value).toBe('fixed');
  });

  it('takes its name from aria-label when there is no visible label', () => {
    render(<Input aria-label="Search" />);
    expect(screen.getByRole('textbox', { name: 'Search' })).toBeTruthy();
  });

  it('two inputs get distinct ids', () => {
    render(<><Input label="A" /><Input label="B" /></>);
    expect(screen.getByLabelText('A').id).not.toBe(screen.getByLabelText('B').id);
  });

  it('a controlled input reports changes', () => {
    function Case() {
      const [value, setValue] = useState('');
      return <Input label="Name" value={value} onChange={(event) => setValue(event.target.value)} />;
    }
    render(<Case />);
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'abc' } });
    expect((screen.getByLabelText('Name') as HTMLInputElement).value).toBe('abc');
  });
});

describe('multiline Input (AC27)', () => {
  it('renders a textarea with the label associated and rows applied', () => {
    render(<Input multiline label="Notes" rows={5} />);
    const control = screen.getByLabelText('Notes') as HTMLTextAreaElement;
    expect(control.tagName).toBe('TEXTAREA');
    expect(control.rows).toBe(5);
  });

  it('defaults to 3 rows', () => {
    render(<Input multiline label="Notes" />);
    expect((screen.getByLabelText('Notes') as HTMLTextAreaElement).rows).toBe(3);
  });

  it('error sets aria-invalid and aria-describedby exactly like the single-line form', () => {
    render(<Input multiline label="Notes" error="Required" />);
    const control = screen.getByLabelText('Notes');
    expect(control.getAttribute('aria-invalid')).toBe('true');
    expect(control.getAttribute('aria-describedby')).toBe(screen.getByText('Required').id);
  });

  it('disabled is not editable', () => {
    render(<Input multiline label="Notes" disabled />);
    expect((screen.getByLabelText('Notes') as HTMLTextAreaElement).disabled).toBe(true);
  });

  it('does not leak the multiline flag to the DOM', () => {
    render(<Input multiline label="Notes" />);
    expect(screen.getByLabelText('Notes').hasAttribute('multiline')).toBe(false);
  });
});

describe('Select (AC13)', () => {
  const options = [
    { value: 'openai', label: 'OpenAI' },
    { value: 'anthropic', label: 'Anthropic' },
  ];

  function SelectCase({ disabled = false, initial = '' }: { disabled?: boolean; initial?: string }) {
    const [value, setValue] = useState(initial);
    return (
      <>
        <Select label="Provider" options={options} value={value} onChange={setValue} placeholder="Pick one" disabled={disabled} />
        <output>{value}</output>
      </>
    );
  }

  it('is a combobox named by its label and shows the placeholder', () => {
    render(<SelectCase />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    expect(trigger.textContent).toContain('Pick one');
  });

  it('shows the controlled value as the selected label', () => {
    render(<SelectCase initial="anthropic" />);
    expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('Anthropic');
  });

  it('opens from the keyboard, lists the options and reports the chosen value through onChange', async () => {
    render(<SelectCase />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    const listbox = await screen.findByRole('listbox');
    expect(within(listbox).getAllByRole('option').map((option) => option.textContent)).toEqual(['OpenAI', 'Anthropic']);
    fireEvent.click(within(listbox).getByRole('option', { name: 'Anthropic' }));
    expect(document.querySelector('output')?.textContent).toBe('anthropic');
    expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('Anthropic');
  });

  it('disabled cannot be opened', () => {
    render(<SelectCase disabled />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' }) as HTMLButtonElement;
    expect(trigger.disabled).toBe(true);
    fireEvent.keyDown(trigger, { key: 'Enter' });
    fireEvent.pointerDown(trigger, { button: 0, pointerType: 'mouse' });
    expect(screen.queryByRole('listbox')).toBeNull();
  });

  it('error links aria-invalid and aria-describedby on the trigger', () => {
    render(<Select label="Provider" options={options} value="" onChange={() => {}} error="Choose a provider" />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    expect(trigger.getAttribute('aria-invalid')).toBe('true');
    expect(trigger.getAttribute('aria-describedby')).toBe(screen.getByText('Choose a provider').id);
  });
});

describe('Checkbox (AC13)', () => {
  function CheckboxCase({ disabled = false, onChange }: { disabled?: boolean; onChange?: (checked: boolean) => void }) {
    const [checked, setChecked] = useState(false);
    return (
      <Checkbox
        label="Accept terms"
        checked={checked}
        disabled={disabled}
        onChange={(next) => {
          onChange?.(next);
          setChecked(next);
        }}
      />
    );
  }

  it('is a checkbox named by its label, and clicking the label toggles it', () => {
    render(<CheckboxCase />);
    const box = screen.getByRole('checkbox', { name: 'Accept terms' });
    expect(box.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(screen.getByText('Accept terms'));
    expect(box.getAttribute('aria-checked')).toBe('true');
  });

  it('reports booleans through onChange and mirrors the controlled value', () => {
    const onChange = vi.fn();
    render(<CheckboxCase onChange={onChange} />);
    const box = screen.getByRole('checkbox');
    // A browser turns Space on a focused <button> into this click; jsdom does not (see file header).
    fireEvent.click(box);
    fireEvent.click(box);
    expect(onChange.mock.calls).toEqual([[true], [false]]);
    expect(box.getAttribute('aria-checked')).toBe('false');
  });

  it('stays what the parent says: an ignored onChange does not flip the control', () => {
    render(<Checkbox label="Pinned" checked={false} onChange={() => {}} />);
    const box = screen.getByRole('checkbox', { name: 'Pinned' });
    fireEvent.click(box);
    expect(box.getAttribute('aria-checked')).toBe('false');
  });

  it('supports indeterminate (aria-checked="mixed") and moves to true on click', () => {
    const onChange = vi.fn();
    render(<Checkbox label="All" checked="indeterminate" onChange={onChange} />);
    const box = screen.getByRole('checkbox', { name: 'All' });
    expect(box.getAttribute('aria-checked')).toBe('mixed');
    fireEvent.click(box);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it('disabled cannot be toggled, by the control or by its label', () => {
    const onChange = vi.fn();
    render(<CheckboxCase disabled onChange={onChange} />);
    const box = screen.getByRole('checkbox') as HTMLButtonElement;
    expect(box.disabled).toBe(true);
    fireEvent.click(box);
    fireEvent.click(screen.getByText('Accept terms'));
    expect(onChange).not.toHaveBeenCalled();
  });

  it('takes its name from aria-label when there is no visible label', () => {
    render(<Checkbox aria-label="Select row 3" checked={false} onChange={() => {}} />);
    expect(screen.getByRole('checkbox', { name: 'Select row 3' })).toBeTruthy();
  });
});

describe('RadioGroup (AC13)', () => {
  const options = [
    { value: 'a', label: 'Alpha' },
    { value: 'b', label: 'Beta' },
    { value: 'c', label: 'Gamma' },
  ];

  function RadioCase({ initial = 'a', disabled = false }: { initial?: string; disabled?: boolean }) {
    const [value, setValue] = useState(initial);
    return (
      <>
        <RadioGroup label="Protocol" options={options} value={value} onChange={setValue} disabled={disabled} />
        <output>{value}</output>
      </>
    );
  }

  it('is a radiogroup named by its label with one radio per option', () => {
    render(<RadioCase />);
    const group = screen.getByRole('radiogroup', { name: 'Protocol' });
    expect(within(group).getAllByRole('radio').map((radio) => radio.getAttribute('aria-checked'))).toEqual(['true', 'false', 'false']);
    expect(screen.getByRole('radio', { name: 'Beta' })).toBeTruthy();
  });

  it('selecting an option reports its value and moves the checked state', () => {
    render(<RadioCase />);
    fireEvent.click(screen.getByRole('radio', { name: 'Gamma' }));
    expect(document.querySelector('output')?.textContent).toBe('c');
    expect(screen.getByRole('radio', { name: 'Gamma' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Alpha' }).getAttribute('aria-checked')).toBe('false');
  });

  it('arrow keys move the selection inside the group', async () => {
    render(<RadioCase />);
    screen.getByRole('radio', { name: 'Alpha' }).focus();
    fireEvent.keyDown(screen.getByRole('radio', { name: 'Alpha' }), { key: 'ArrowDown' });
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('b'));
    fireEvent.keyDown(screen.getByRole('radio', { name: 'Beta' }), { key: 'ArrowDown' });
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('c'));
    // wraps around at the end
    fireEvent.keyDown(screen.getByRole('radio', { name: 'Gamma' }), { key: 'ArrowDown' });
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('a'));
    fireEvent.keyDown(screen.getByRole('radio', { name: 'Alpha' }), { key: 'ArrowUp' });
    await waitFor(() => expect(document.querySelector('output')?.textContent).toBe('c'));
  });

  // Not asserted: which element is the tab stop. Radix makes the radiogroup container the stop (tabindex 0, all
  // radios -1 until it receives focus and forwards it, observed in jsdom), and a manual `.focus()` does not run
  // that forwarding. Tab order is a real-WebView check (Phase 8).

  it('disabled group cannot change the value', () => {
    render(<RadioCase disabled />);
    fireEvent.click(screen.getByRole('radio', { name: 'Beta' }));
    expect(document.querySelector('output')?.textContent).toBe('a');
    for (const radio of screen.getAllByRole('radio')) expect((radio as HTMLButtonElement).disabled).toBe(true);
  });

  it('a disabled option cannot be selected but its siblings can', () => {
    function Case() {
      const [value, setValue] = useState('a');
      return (
        <>
          <RadioGroup
            label="Protocol"
            value={value}
            onChange={setValue}
            options={[{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta', disabled: true }, { value: 'c', label: 'Gamma' }]}
          />
          <output>{value}</output>
        </>
      );
    }
    render(<Case />);
    fireEvent.click(screen.getByRole('radio', { name: 'Beta' }));
    expect(document.querySelector('output')?.textContent).toBe('a');
    fireEvent.click(screen.getByRole('radio', { name: 'Gamma' }));
    expect(document.querySelector('output')?.textContent).toBe('c');
  });
});

describe('Switch (AC13)', () => {
  function SwitchCase({ disabled = false, onChange }: { disabled?: boolean; onChange?: (checked: boolean) => void }) {
    const [checked, setChecked] = useState(false);
    return (
      <Switch
        label="Auto update"
        checked={checked}
        disabled={disabled}
        onChange={(next) => {
          onChange?.(next);
          setChecked(next);
        }}
      />
    );
  }

  it('is role=switch, named by its label, and aria-checked follows the state', () => {
    render(<SwitchCase />);
    const control = screen.getByRole('switch', { name: 'Auto update' });
    expect(control.getAttribute('aria-checked')).toBe('false');
    // Space on a focused <button> is dispatched by a browser as this click (see file header).
    fireEvent.click(control);
    expect(control.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByText('Auto update'));
    expect(control.getAttribute('aria-checked')).toBe('false');
  });

  it('Enter does not toggle: only Space does, as for a native switch', () => {
    const onChange = vi.fn();
    render(<SwitchCase onChange={onChange} />);
    const control = screen.getByRole('switch');
    fireEvent.keyDown(control, { key: 'Enter' });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('disabled cannot be toggled', () => {
    const onChange = vi.fn();
    render(<SwitchCase disabled onChange={onChange} />);
    const control = screen.getByRole('switch') as HTMLButtonElement;
    expect(control.disabled).toBe(true);
    fireEvent.click(control);
    fireEvent.click(screen.getByText('Auto update'));
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe('display components (AC14)', () => {
  it('Card renders its slots in order and has no interactive role', () => {
    render(
      <Card data-testid="card">
        <Card.Header>Providers</Card.Header>
        <Card.Body>body text</Card.Body>
        <Card.Footer>footer text</Card.Footer>
      </Card>,
    );
    const card = screen.getByTestId('card');
    expect(Array.from(card.children).map((child) => child.textContent)).toEqual(['Providers', 'body text', 'footer text']);
    expect(card.hasAttribute('role')).toBe(false);
    expect(card.hasAttribute('tabindex')).toBe(false);
  });

  it('Card works with only Header and Body', () => {
    render(<Card data-testid="card"><Card.Header>H</Card.Header><Card.Body>B</Card.Body></Card>);
    expect(screen.getByTestId('card').children).toHaveLength(2);
  });

  it('Loading is role=status; its visible text is announced as content, with no competing aria-label', () => {
    render(<Loading label="Syncing models" />);
    // `status` does not take its name from content (ARIA), so the text is checked as content, not as a name.
    const status = screen.getByRole('status');
    expect(status.textContent).toBe('Syncing models');
    expect(status.hasAttribute('aria-label')).toBe(false);
    expect(status.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  });

  it('Loading without visible text is named by aria-label', () => {
    render(<Loading aria-label="Loading providers" />);
    const status = screen.getByRole('status', { name: 'Loading providers' });
    expect(status.textContent).toBe('');
  });

  it('Loading sizes map to different spinner size classes', () => {
    const sizes = ['sm', 'md', 'lg'] as const;
    const classes = sizes.map((size) => {
      const { unmount } = render(<Loading size={size} aria-label="x" />);
      const value = screen.getByRole('status').querySelector('svg')?.getAttribute('class') ?? '';
      unmount();
      return value.split(/\s+/).find((name) => name.startsWith('size-'));
    });
    expect(classes).toEqual(['size-4', 'size-6', 'size-8']);
  });

  it('Loading and a Loading-less aria-label pair is a type error', () => {
    // @ts-expect-error neither label nor aria-label
    const element = <Loading />;
    expect(element).toBeTruthy();
  });

  it('Divider is a horizontal separator by default, with no aria-orientation', () => {
    render(<Divider />);
    const separator = screen.getByRole('separator');
    expect(separator.hasAttribute('aria-orientation')).toBe(false);
  });

  it('Divider vertical sets aria-orientation="vertical"', () => {
    render(<Divider orientation="vertical" />);
    expect(screen.getByRole('separator').getAttribute('aria-orientation')).toBe('vertical');
  });
});

describe('Tabs (AC26)', () => {
  function TabsCase({ onValueChange }: { onValueChange?: (value: string) => void }) {
    const [value, setValue] = useState('a');
    return (
      <Tabs
        value={value}
        onValueChange={(next) => {
          onValueChange?.(next);
          setValue(next);
        }}
      >
        <Tabs.List aria-label="Settings sections">
          <Tabs.Trigger value="a">Alpha</Tabs.Trigger>
          <Tabs.Trigger value="b">Beta</Tabs.Trigger>
          <Tabs.Trigger value="c" disabled>Gamma</Tabs.Trigger>
          <Tabs.Trigger value="d">Delta</Tabs.Trigger>
        </Tabs.List>
        <Tabs.Content value="a">panel a</Tabs.Content>
        <Tabs.Content value="b">panel b</Tabs.Content>
        <Tabs.Content value="c">panel c</Tabs.Content>
        <Tabs.Content value="d">panel d</Tabs.Content>
      </Tabs>
    );
  }

  it('exposes tablist, tab and tabpanel roles with aria-selected on the active tab only', () => {
    render(<TabsCase />);
    expect(screen.getByRole('tablist', { name: 'Settings sections' })).toBeTruthy();
    const tabs = screen.getAllByRole('tab');
    expect(tabs.map((tab) => tab.getAttribute('aria-selected'))).toEqual(['true', 'false', 'false', 'false']);
    expect(screen.getByRole('tabpanel').textContent).toBe('panel a');
  });

  it('the panel is labelled by its tab and the tab controls the panel', () => {
    render(<TabsCase />);
    const tab = screen.getByRole('tab', { name: 'Alpha' });
    const panel = screen.getByRole('tabpanel');
    expect(panel.getAttribute('aria-labelledby')).toBe(tab.id);
    expect(tab.getAttribute('aria-controls')).toBe(panel.id);
    expect(screen.getByRole('tabpanel', { name: 'Alpha' })).toBe(panel);
  });

  it('only the active panel is present: inactive panels are not rendered', () => {
    render(<TabsCase />);
    expect(screen.getAllByRole('tabpanel')).toHaveLength(1);
    expect(screen.queryByText('panel b')).toBeNull();
  });

  it('selecting a tab shows its panel and hides the previous one', () => {
    const onValueChange = vi.fn();
    render(<TabsCase onValueChange={onValueChange} />);
    // Radix selects on mousedown, not click.
    fireEvent.mouseDown(screen.getByRole('tab', { name: 'Beta' }), { button: 0 });
    expect(onValueChange).toHaveBeenCalledWith('b');
    expect(screen.getByRole('tab', { name: 'Beta' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tab', { name: 'Alpha' }).getAttribute('aria-selected')).toBe('false');
    expect(screen.getByRole('tabpanel').textContent).toBe('panel b');
    expect(screen.queryByText('panel a')).toBeNull();
  });

  it('arrow keys move focus and the selection, skip a disabled tab and wrap', async () => {
    render(<TabsCase />);
    const alpha = screen.getByRole('tab', { name: 'Alpha' });
    alpha.focus();
    fireEvent.keyDown(alpha, { key: 'ArrowRight' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Beta' }).getAttribute('aria-selected')).toBe('true'));
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Beta' }));

    // Gamma is disabled: ArrowRight goes straight to Delta.
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Beta' }), { key: 'ArrowRight' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Delta' }).getAttribute('aria-selected')).toBe('true'));
    expect(screen.getByRole('tabpanel').textContent).toBe('panel d');

    // wrap to the first tab
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Delta' }), { key: 'ArrowRight' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Alpha' }).getAttribute('aria-selected')).toBe('true'));

    fireEvent.keyDown(screen.getByRole('tab', { name: 'Alpha' }), { key: 'ArrowLeft' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Delta' }).getAttribute('aria-selected')).toBe('true'));
  });

  it('Home and End jump to the first and last enabled tab', async () => {
    render(<TabsCase />);
    const alpha = screen.getByRole('tab', { name: 'Alpha' });
    alpha.focus();
    fireEvent.keyDown(alpha, { key: 'End' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Delta' }).getAttribute('aria-selected')).toBe('true'));
    fireEvent.keyDown(screen.getByRole('tab', { name: 'Delta' }), { key: 'Home' });
    await waitFor(() => expect(screen.getByRole('tab', { name: 'Alpha' }).getAttribute('aria-selected')).toBe('true'));
  });

  it('a disabled tab cannot be selected', () => {
    const onValueChange = vi.fn();
    render(<TabsCase onValueChange={onValueChange} />);
    fireEvent.mouseDown(screen.getByRole('tab', { name: 'Gamma' }), { button: 0 });
    expect(onValueChange).not.toHaveBeenCalled();
    expect(screen.getByRole('tab', { name: 'Gamma' }).getAttribute('aria-selected')).toBe('false');
  });

  // Not asserted: the tab stop. Radix Tabs puts tabindex 0 on the tablist and -1 on every tab (observed), then
  // forwards focus to the active tab; that forwarding needs a real focus event sequence (Phase 8).

  it('is controlled: without a state update the selection does not change', () => {
    render(
      <Tabs value="a" onValueChange={() => {}}>
        <Tabs.List aria-label="x"><Tabs.Trigger value="a">Alpha</Tabs.Trigger><Tabs.Trigger value="b">Beta</Tabs.Trigger></Tabs.List>
        <Tabs.Content value="a">panel a</Tabs.Content>
        <Tabs.Content value="b">panel b</Tabs.Content>
      </Tabs>,
    );
    fireEvent.mouseDown(screen.getByRole('tab', { name: 'Beta' }), { button: 0 });
    expect(screen.getByRole('tab', { name: 'Alpha' }).getAttribute('aria-selected')).toBe('true');
    expect(screen.getByRole('tabpanel').textContent).toBe('panel a');
  });
});

describe('Stack (AC27)', () => {
  // The closed table: each step maps to one complete Tailwind class that names a --spacing-N token step.
  const GAPS = [0, 1, 2, 3, 4, 5, 6, 8, 10, 12] as const;

  it.each(GAPS)('gap=%i renders exactly gap-%i', (gap) => {
    render(<Stack gap={gap} data-testid="stack" />);
    const classes = screen.getByTestId('stack').className.split(/\s+/);
    expect(classes.filter((name) => name.startsWith('gap-'))).toEqual([`gap-${gap}`]);
  });

  it('every gap step has a spacing token of the same number', () => {
    const { light } = themeTokens();
    for (const gap of GAPS) expect(light.has(`--spacing-${gap}`), `--spacing-${gap}`).toBe(true);
  });

  it('every gap class compiles to a spacing rule (the class literals are visible to Tailwind)', async () => {
    const css = await compileUi(GAPS.map((gap) => `gap-${gap}`));
    for (const gap of GAPS) {
      // Tailwind folds multiplier 0 to a literal `0px` and multiplier 1 to the bare unit; the rest keep the multiple.
      const value =
        gap === 0 ? '0px' : gap === 1 ? 'var\\(--spacing-unit\\)' : `calc\\(var\\(--spacing-unit\\)\\s*\\*\\s*${gap}\\)`;
      expect(css, `gap-${gap}`).toMatch(new RegExp(`\\.gap-${gap}\\s*\\{[^}]*gap:\\s*${value}`));
    }
  });

  it('direction picks the flex axis; vertical is the default', () => {
    const { rerender } = render(<Stack gap={2} data-testid="stack" />);
    expect(screen.getByTestId('stack').className.split(/\s+/)).toContain('flex-col');
    rerender(<Stack gap={2} direction="horizontal" data-testid="stack" />);
    const classes = screen.getByTestId('stack').className.split(/\s+/);
    expect(classes).toContain('flex-row');
    expect(classes).not.toContain('flex-col');
  });

  it('a value outside the enum produces no gap class instead of an arbitrary one', () => {
    for (const bad of [7, 99, 4.5, NaN, '4', 'gap-[13px]', 'constructor', '__proto__', 'toString', -1, undefined]) {
      const { unmount } = render(<Stack {...untyped<{ gap: 0 }>({ gap: bad })} data-testid="stack" />);
      const classes = screen.getByTestId('stack').className.split(/\s+/);
      expect(classes.filter((name) => name.startsWith('gap-')), String(bad)).toEqual([]);
      unmount();
    }
  });

  it('renders its children in order', () => {
    render(<Stack gap={2} data-testid="stack"><span>one</span><span>two</span></Stack>);
    expect(Array.from(screen.getByTestId('stack').children).map((child) => child.textContent)).toEqual(['one', 'two']);
  });

  it('an out-of-enum gap is a type error', () => {
    // @ts-expect-error 7 is not a gap step
    const element = <Stack gap={7} />;
    expect(element).toBeTruthy();
  });
});

describe('no component takes an inline style (FR-005 §7.2.1)', () => {
  const bad = { style: { color: 'red', padding: '13px' } };

  it('drops a style prop that untyped callers pass, on every component', () => {
    const cases: Array<[string, () => HTMLElement]> = [
      ['Button', () => { render(<Button {...untyped<{ children: string }>({ ...bad, children: 'b' })} />); return screen.getByRole('button'); }],
      ['Input', () => { render(<Input {...untyped<{ label: string }>({ ...bad, label: 'i' })} />); return screen.getByLabelText('i'); }],
      ['multiline Input', () => { render(<Input {...untyped<{ label: string }>({ ...bad, label: 'm', multiline: true })} />); return screen.getByLabelText('m'); }],
      ['Card', () => { render(<Card {...untyped<object>({ ...bad, 'data-testid': 'x' })} />); return screen.getByTestId('x'); }],
      ['Card.Header', () => { render(<Card.Header {...untyped<object>({ ...bad, 'data-testid': 'x' })} />); return screen.getByTestId('x'); }],
      ['Card.Body', () => { render(<Card.Body {...untyped<object>({ ...bad, 'data-testid': 'x' })} />); return screen.getByTestId('x'); }],
      ['Card.Footer', () => { render(<Card.Footer {...untyped<object>({ ...bad, 'data-testid': 'x' })} />); return screen.getByTestId('x'); }],
      ['Loading', () => { render(<Loading {...untyped<{ label: string }>({ ...bad, label: 'l' })} />); return screen.getByRole('status'); }],
      ['Divider', () => { render(<Divider {...untyped<object>({ ...bad })} />); return screen.getByRole('separator'); }],
      ['Stack', () => { render(<Stack {...untyped<{ gap: 2 }>({ ...bad, gap: 2, 'data-testid': 'x' })} />); return screen.getByTestId('x'); }],
      ['Tabs', () => { render(<Tabs {...untyped<{ value: string; onValueChange: () => void }>({ ...bad, value: 'a', onValueChange: () => {}, 'data-testid': 'x' })}><Tabs.List aria-label="t"><Tabs.Trigger value="a">A</Tabs.Trigger></Tabs.List></Tabs>); return screen.getByTestId('x'); }],
      ['Tabs.List', () => { render(<Tabs value="a" onValueChange={() => {}}><Tabs.List {...untyped<{ 'aria-label': string }>({ ...bad, 'aria-label': 't' })}><Tabs.Trigger value="a">A</Tabs.Trigger></Tabs.List></Tabs>); return screen.getByRole('tablist'); }],
      ['Tabs.Trigger', () => { render(<Tabs value="a" onValueChange={() => {}}><Tabs.List aria-label="t"><Tabs.Trigger {...untyped<{ value: string; children: string }>({ ...bad, value: 'a', children: 'A' })} /></Tabs.List></Tabs>); return screen.getByRole('tab'); }],
      ['Tabs.Content', () => { render(<Tabs value="a" onValueChange={() => {}}><Tabs.List aria-label="t"><Tabs.Trigger value="a">A</Tabs.Trigger></Tabs.List><Tabs.Content {...untyped<{ value: string }>({ ...bad, value: 'a' })} /></Tabs>); return screen.getByRole('tabpanel'); }],
    ];
    for (const [name, mount] of cases) {
      const element = mount();
      // Radix writes its own `outline: none` on the roving-focus containers (tablist, radiogroup); that is the
      // primitive's, not the caller's. What must not survive is anything the caller passed.
      expect(element.getAttribute('style') ?? '', name).not.toMatch(/red|13px|padding|color/);
      cleanup();
    }
  });
});

describe('focus ring (AC24)', () => {
  const FOCUS = /(^|\s)focus-visible:outline-2(\s|$)/;

  /** One of everything interactive, in a state where all of it is in the DOM. */
  function Everything() {
    return (
      <>
        <Button>b</Button>
        <Input label="i" />
        <Input multiline label="m" />
        <Select label="s" options={[{ value: 'x', label: 'X' }]} value="" onChange={() => {}} />
        <Checkbox label="c" checked={false} onChange={() => {}} />
        <RadioGroup label="r" options={[{ value: 'x', label: 'X' }]} value="x" onChange={() => {}} />
        <Switch label="w" checked={false} onChange={() => {}} />
        <Tabs value="a" onValueChange={() => {}}>
          <Tabs.List aria-label="t"><Tabs.Trigger value="a">A</Tabs.Trigger></Tabs.List>
          <Tabs.Content value="a">panel</Tabs.Content>
        </Tabs>
      </>
    );
  }

  it('every interactive element carries a focus-visible outline', () => {
    render(<Everything />);
    const interactive = [
      ...screen.getAllByRole('button'),
      ...screen.getAllByRole('textbox'),
      ...screen.getAllByRole('combobox'),
      ...screen.getAllByRole('checkbox'),
      ...screen.getAllByRole('radio'),
      ...screen.getAllByRole('switch'),
      ...screen.getAllByRole('tab'),
      screen.getByRole('tabpanel'),
    ];
    // button, 2 text fields, combobox, checkbox, radio, switch, tab + panel
    expect(new Set(interactive).size).toBeGreaterThanOrEqual(9);
    for (const element of new Set(interactive)) {
      const label = `${element.tagName} role=${element.getAttribute('role') ?? '-'}`;
      expect(element.className, label).toMatch(FOCUS);
      expect(element.className, label).toContain('focus-visible:outline-focus-ring');
    }
  });

  it('no rendered element removes the outline', () => {
    const { container } = render(<Everything />);
    for (const element of container.querySelectorAll('*')) {
      const className = element.getAttribute('class') ?? '';
      expect(className, element.tagName).not.toMatch(/(^|[\s:])outline-none(\s|$)/);
    }
  });

  it('the shared ring classes are real Tailwind utilities that read --color-focus-ring on :focus-visible', async () => {
    const classes = [...FOCUS_RING.split(' '), ...FOCUS_RING_INSET.split(' ')];
    const css = await compileUi(classes);
    // Each utility must sit under :focus-visible (never on the bare element) and carry the exact declaration.
    expect(css).toMatch(/\.focus-visible\\:outline-focus-ring:focus-visible\s*\{\s*outline-color:\s*var\(--color-focus-ring\);?\s*\}/);
    expect(css).toMatch(/\.focus-visible\\:outline-2:focus-visible\s*\{\s*outline-style:\s*var\(--tw-outline-style\);\s*outline-width:\s*2px;?\s*\}/);
    expect(css).toMatch(/\.focus-visible\\:outline-offset-2:focus-visible\s*\{\s*outline-offset:\s*2px;?\s*\}/);
    expect(css).toMatch(/\.focus-visible\\:-outline-offset-2:focus-visible\s*\{\s*outline-offset:\s*calc\(2px \* -1\);?\s*\}/);
    // `outline-style` reads --tw-outline-style, which must default to solid or the outline would not draw.
    expect(css).toMatch(/@property --tw-outline-style\s*\{[^}]*initial-value:\s*solid/);
  });

  it('the ring colour meets 3:1 on the surfaces components sit on, in both themes (WCAG 1.4.11)', () => {
    const { light, dark } = themeTokens();
    for (const [name, tokens] of [['light', light], ['dark', dark]] as const) {
      for (const surface of ['bg-primary', 'bg-secondary', 'bg-tertiary']) {
        const ratio = contrast(tokens.get('--color-focus-ring')!, tokens.get(`--color-${surface}`)!);
        expect(ratio, `${name} focus-ring on ${surface}`).toBeGreaterThanOrEqual(3);
      }
    }
  });
});

describe('component colour pairs (AC20 holds for what P4 actually uses)', () => {
  // Foreground/background pairs the variants above put on screen. All are checked in both themes.
  const pairs: Array<[string, string, string, number]> = [
    ['primary button', 'on-primary', 'primary', 4.5],
    ['primary button hover', 'on-primary', 'primary-hover', 4.5],
    ['primary button active', 'on-primary', 'primary-active', 4.5],
    ['danger button', 'on-error', 'error', 4.5],
    ['danger button hover', 'on-error', 'error-text', 4.5],
    ['secondary button', 'text-primary', 'bg-tertiary', 4.5],
    ['secondary button hover', 'text-primary', 'border', 4.5],
    ['ghost button hover', 'text-primary', 'bg-tertiary', 4.5],
    ['link', 'primary', 'bg-primary', 4.5],
    ['error text', 'error-text', 'bg-primary', 4.5],
    ['control border', 'border-control', 'bg-primary', 3],
    ['error border', 'error', 'bg-primary', 3],
    ['success border', 'success', 'bg-primary', 3],
    ['inactive tab text', 'text-secondary', 'bg-primary', 4.5],
    ['card text', 'text-primary', 'bg-secondary', 4.5],
    ['card secondary text', 'text-secondary', 'bg-secondary', 4.5],
    ['loading text', 'text-secondary', 'bg-primary', 4.5],
    ['loading spinner', 'primary', 'bg-primary', 3],
  ];

  it.each(pairs)('%s: --color-%s on --color-%s >= %f:1 in light and dark', (_name, fg, bg, min) => {
    const { light, dark } = themeTokens();
    for (const [theme, tokens] of [['light', light], ['dark', dark]] as const) {
      const ratio = contrast(tokens.get(`--color-${fg}`)!, tokens.get(`--color-${bg}`)!);
      expect(ratio, `${theme}`).toBeGreaterThanOrEqual(min);
    }
  });
});

describe('component class literals compile (D066: unscanned class = no CSS)', () => {
  it('the utilities the components rely on exist in the real Tailwind build', async () => {
    const css = await compileUi([
      'bg-primary', 'text-primary-foreground', 'enabled:hover:bg-primary-hover', 'bg-destructive', 'text-destructive-foreground',
      'enabled:hover:bg-error-text', 'bg-transparent', 'border-input', 'border-error', 'border-success', 'opacity-disabled',
      'disabled:opacity-disabled', 'aria-busy:cursor-progress', 'text-error-text', 'placeholder:text-muted-foreground',
      'animate-spin', 'h-8', 'h-9', 'h-10', 'bg-divider', 'data-[state=checked]:bg-primary', 'w-(--radix-select-trigger-width)',
      'max-h-(--radix-select-content-available-height)', 'bg-popover', 'bg-card', 'border-border',
    ]);
    for (const needle of [
      '.bg-primary', '.bg-destructive', '.bg-transparent', '.border-input', '.border-error', '.border-success',
      '.animate-spin', '.bg-divider', '.bg-popover', '.bg-card', '.border-border', '.text-error-text',
    ]) {
      expect(css, needle).toContain(needle);
    }
    expect(css).toMatch(/@keyframes spin/);
    expect(css).toMatch(/--animate-spin:\s*spin 1s linear infinite/);
    expect(css).toMatch(/width:\s*var\(--radix-select-trigger-width\)/);
    expect(css).toMatch(/max-height:\s*var\(--radix-select-content-available-height\)/);
    // opacity-disabled must read the token, not a literal
    expect(css).toMatch(/\.opacity-disabled\s*\{\s*opacity:\s*var\(--opacity-disabled\)/);
  });
});

describe('visible label wins over aria-label and aria-labelledby (D069, WCAG 2.5.3)', () => {
  // Untyped callers (module code, D067) can pass all three. The visible text must be the name; the aria
  // attributes are dropped so they cannot override it.
  const conflict = { label: 'Visible', 'aria-label': 'Hidden name', 'aria-labelledby': 'elsewhere' };

  function expectNamedByVisible(control: HTMLElement) {
    expect(control.hasAttribute('aria-label')).toBe(false);
    expect(control.getAttribute('aria-labelledby')).not.toBe('elsewhere');
  }

  it('Input', () => {
    render(<Input {...untyped<{ label: string }>(conflict)} />);
    const input = screen.getByRole('textbox', { name: 'Visible' });
    expectNamedByVisible(input);
  });

  it('multiline Input', () => {
    render(<Input {...untyped<{ label: string }>({ ...conflict, multiline: true })} />);
    expectNamedByVisible(screen.getByRole('textbox', { name: 'Visible' }));
  });

  it('Select', () => {
    render(<Select {...untyped<React.ComponentProps<typeof Select>>({ ...conflict, options: [], value: '', onChange: () => {} })} />);
    expectNamedByVisible(screen.getByRole('combobox', { name: 'Visible' }));
  });

  it('Checkbox', () => {
    render(<Checkbox {...untyped<React.ComponentProps<typeof Checkbox>>({ ...conflict, checked: false, onChange: () => {} })} />);
    expectNamedByVisible(screen.getByRole('checkbox', { name: 'Visible' }));
  });

  it('Switch', () => {
    render(<Switch {...untyped<React.ComponentProps<typeof Switch>>({ ...conflict, checked: false, onChange: () => {} })} />);
    expectNamedByVisible(screen.getByRole('switch', { name: 'Visible' }));
  });

  it('RadioGroup is labelled by its own visible label element, not the caller id', () => {
    render(
      <RadioGroup
        {...untyped<React.ComponentProps<typeof RadioGroup>>({ ...conflict, options: [{ value: 'a', label: 'A' }], value: '', onChange: () => {} })}
      />,
    );
    const group = screen.getByRole('radiogroup', { name: 'Visible' });
    expect(group.hasAttribute('aria-label')).toBe(false);
    expect(document.getElementById(group.getAttribute('aria-labelledby')!)?.textContent).toBe('Visible');
  });

  it('Loading', () => {
    render(<Loading {...untyped<{ label: string }>({ label: 'Visible', 'aria-label': 'Hidden name' })} />);
    const status = screen.getByRole('status');
    expect(status.hasAttribute('aria-label')).toBe(false);
    expect(status.textContent).toBe('Visible');
  });

  it('an empty label is not visible text: the aria name is used', () => {
    render(<Checkbox {...untyped<React.ComponentProps<typeof Checkbox>>({ label: '', 'aria-label': 'Select row', checked: false, onChange: () => {} })} />);
    expect(screen.getByRole('checkbox', { name: 'Select row' })).toBeTruthy();
    expect(document.querySelector('label')).toBeNull();
  });

  it('without a visible label, aria-labelledby wins over aria-label (accname order)', () => {
    render(
      <>
        <span id="ref">Referenced</span>
        <Switch {...untyped<React.ComponentProps<typeof Switch>>({ 'aria-label': 'Plain', 'aria-labelledby': 'ref', checked: false, onChange: () => {} })} />
      </>,
    );
    const control = screen.getByRole('switch', { name: 'Referenced' });
    expect(control.hasAttribute('aria-label')).toBe(false);
  });

  it('Named is mutually exclusive in the types', () => {
    // These are checked by `tsc` only when tests are included (see spec §7.2.4); they do not run any code.
    const never = () => {
      // @ts-expect-error label and aria-label together
      <Checkbox label="x" aria-label="y" checked onChange={() => {}} />;
      // @ts-expect-error label and aria-labelledby together
      <Switch label="x" aria-labelledby="y" checked onChange={() => {}} />;
      // @ts-expect-error aria-label and aria-labelledby together
      <Input aria-label="x" aria-labelledby="y" />;
      // @ts-expect-error no name at all
      <Select options={[]} value="" onChange={() => {}} />;
    };
    expect(typeof never).toBe('function');
  });
});

describe('Select value outside options (AC13)', () => {
  const options = [
    { value: 'openai', label: 'OpenAI' },
    { value: 'anthropic', label: 'Anthropic' },
  ];

  it('shows the placeholder for a value that is not in options, and does not pick another item', () => {
    const onChange = vi.fn();
    render(<Select label="Provider" options={options} value="gone" onChange={onChange} placeholder="Pick one" />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    expect(trigger.textContent).toContain('Pick one');
    expect(trigger.textContent).not.toContain('OpenAI');
    expect(trigger.textContent).not.toContain('Anthropic');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('returns to the placeholder when the selected option is removed, without selecting a neighbour', () => {
    const onChange = vi.fn();
    const { rerender } = render(<Select label="Provider" options={options} value="anthropic" onChange={onChange} placeholder="Pick one" />);
    expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('Anthropic');
    rerender(<Select label="Provider" options={[options[0]!]} value="anthropic" onChange={onChange} placeholder="Pick one" />);
    const trigger = screen.getByRole('combobox', { name: 'Provider' });
    expect(trigger.textContent).toContain('Pick one');
    expect(trigger.textContent).not.toContain('OpenAI');
    expect(onChange).not.toHaveBeenCalled();
  });

  it('shows the item again when the option comes back', () => {
    const onChange = vi.fn();
    const { rerender } = render(<Select label="Provider" options={[options[0]!]} value="anthropic" onChange={onChange} placeholder="Pick one" />);
    expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('Pick one');
    rerender(<Select label="Provider" options={options} value="anthropic" onChange={onChange} placeholder="Pick one" />);
    expect(screen.getByRole('combobox', { name: 'Provider' }).textContent).toContain('Anthropic');
  });
});
