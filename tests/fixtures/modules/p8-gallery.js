// FR-005 P8 real-WebView gallery: every overlay and keyboard-driven control that no shipped page nests, rendered
// through the frozen `host.ui` (the same objects a module receives). Used only by src-tauri/src/smoke/ui_keys.rs.
export default function createModule(host) {
  const React = host.React;
  const h = React.createElement;
  const ui = host.ui;
  const LONG = 'Unbroken-token-' + 'abcdefghij'.repeat(12) + ' followed by ordinary words that must wrap inside the card.';
  const FRUITS = [
    { value: 'apple', label: 'Apple' },
    { value: 'banana', label: 'Banana' },
    { value: 'cherry', label: 'Cherry' },
  ];

  function DialogBody() {
    const [fruit, setFruit] = React.useState('apple');
    return h(
      ui.Stack,
      { gap: 3 },
      h('p', { id: 'g-dialog-long' }, LONG),
      h(ui.Select, { label: 'Dialog fruit', options: FRUITS, value: fruit, onChange: setFruit }),
      h('span', { id: 'g-dialog-fruit' }, 'dialog fruit=' + fruit),
      h(ui.Tooltip, { content: 'Tooltip inside the dialog' }, h(ui.Button, { id: 'g-dialog-tip', variant: 'secondary' }, 'Hover or focus me')),
      h(ui.Button, { id: 'g-dialog-toast', variant: 'secondary', onClick: () => ui.toast.success('Toast raised while a dialog is open: ' + LONG) }, 'Toast over dialog'),
    );
  }

  function LongDialogBody({ release }) {
    const [fruit, setFruit] = React.useState('apple');
    return h('form', { id: 'g-modal-form', onSubmit: event => event.preventDefault() },
      h(ui.Stack, { gap: 4 },
        h(ui.Input, { id: 'g-modal-first', label: 'First argument', defaultValue: 'Start of the actual form content' }),
        h('pre', { id: 'g-modal-pre', className: 'max-h-60 overflow-auto whitespace-pre-wrap rounded-md border border-edge-control p-3 text-xs' }, JSON.stringify({ request: LONG.repeat(8) }, null, 2)),
        ...Array.from({ length: 16 }, (_, index) => h('p', { key: index }, 'Argument review paragraph ' + index + ': ' + LONG)),
        h(ui.Select, { label: 'Long form fruit', options: FRUITS, value: fruit, onChange: setFruit }),
        h(ui.Input, { id: 'g-modal-last', label: 'Last argument', defaultValue: 'End of the actual form content' }),
        h(ui.Button, { id: 'g-modal-release', variant: 'secondary', onClick: release }, 'Finish pending confirmation'),
      ),
    );
  }

  function openLongDialog() {
    let finish;
    ui.Modal.confirm({
      title: 'Review long attachment and tool arguments — ' + LONG,
      content: h(LongDialogBody, { release: () => finish?.() }),
      cancelText: 'Back to arguments',
      confirmText: 'Confirm reviewed request',
      onConfirm: () => new Promise(resolve => { finish = resolve; }),
    });
  }

  function Component() {
    const [tab, setTab] = React.useState('one');
    const [radio, setRadio] = React.useState('b');
    const [fruit, setFruit] = React.useState('');
    const [on, setOn] = React.useState(false);
    const [checked, setChecked] = React.useState(false);
    const [longValue, setLongValue] = React.useState('zh');
    return h(
      ui.Stack,
      { gap: 4, 'data-testid': 'p8-gallery' },
      h('p', { id: 'g-long' }, LONG),
      h(
        ui.Tabs,
        { value: tab, onValueChange: setTab },
        h(ui.Tabs.List, { 'aria-label': 'Gallery tabs' },
          h(ui.Tabs.Trigger, { value: 'one' }, 'One'),
          h(ui.Tabs.Trigger, { value: 'two' }, 'Two'),
          h(ui.Tabs.Trigger, { value: 'three' }, 'Three')),
        h(ui.Tabs.Content, { value: 'one' }, 'panel one'),
        h(ui.Tabs.Content, { value: 'two' }, 'panel two'),
        h(ui.Tabs.Content, { value: 'three' }, 'panel three'),
      ),
      h(ui.Radio, { label: 'Gallery radio', value: radio, onChange: setRadio, options: [
        { value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }, { value: 'c', label: 'Gamma' } ] }),
      h(ui.Select, { label: 'Gallery fruit', options: FRUITS, value: fruit, placeholder: 'Pick a fruit', onChange: setFruit }),
      h(ui.Switch, { checked: on, onChange: setOn, label: 'Gallery switch' }),
      h(ui.Checkbox, { checked, onChange: setChecked, label: 'Gallery checkbox' }),
      h(ui.Tooltip, { content: 'Tooltip on the page' }, h(ui.Button, { id: 'g-tip', variant: 'secondary' }, 'Tooltip trigger')),
      h(ui.Button, { id: 'g-open', onClick: () => ui.Modal.confirm({ title: 'Gallery dialog', content: h(DialogBody), confirmText: 'OK', cancelText: 'Cancel' }) }, 'Open dialog'),
      h(ui.Button, { id: 'g-toast', variant: 'secondary', onClick: () => ui.toast.error('Gallery error toast: ' + LONG) }, 'Raise error toast'),
      h(ui.Button, { id: 'g-open-prose', variant: 'secondary', onClick: () => ui.Modal.confirm({ title: 'Prose dialog', content: h('p', { id: 'g-prose-long' }, 'Ordinary words only. '.repeat(40)), confirmText: 'OK', cancelText: 'Cancel' }) }, 'Open prose dialog'),
      // AC24 focus-ring targets, appended last so the earlier tab sequences in ui_keys.rs keep their order.
      h(ui.Button, { id: 'g-ghost', variant: 'ghost' }, 'Ghost button'),
      h(ui.Button, { id: 'g-link', variant: 'link' }, 'Link button'),
      h(ui.Input, { id: 'g-input-error', label: 'Gallery input with error', value: '', onChange: () => {}, error: 'This field is required' }),
      h(ui.Checkbox, { checked: true, onChange: () => {}, label: 'Gallery checkbox checked' }),
      h(ui.Button, { id: 'g-toast-long', variant: 'secondary', onClick: () => ui.toast.error('Focus-ring toast: ' + LONG, { duration: 60000 }) }, 'Raise long error toast'),
      h(ui.Button, { id: 'g-open-long-form', variant: 'secondary', onClick: openLongDialog }, 'Open long consumer dialog'),
      h(ui.Select, { id: 'g-long-select', label: 'Long selected identifier', value: longValue, onChange: setLongValue, options: [
        { value: 'zh', label: '中文长模型名称用于检查完整值可访问与控件边界'.repeat(8) },
        { value: 'en', label: 'LongEnglishIdentifierSupercalifragilisticexpialidocious'.repeat(8) },
      ] }),
      h(ui.Input, { id: 'g-input-ui', label: 'Ordinary notes', multiline: true, defaultValue: 'Ordinary form' }),
      h(ui.Input, { id: 'g-input-reading', label: 'Reading draft', multiline: true, className: 'type-reading', defaultValue: 'Reading draft' }),
      h(ui.Input, { id: 'g-input-code', label: 'Code draft', multiline: true, className: 'type-code', defaultValue: '{"code":true}' }),
    );
  }
  return { Component };
}
