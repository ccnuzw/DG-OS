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

  function Component() {
    const [tab, setTab] = React.useState('one');
    const [radio, setRadio] = React.useState('b');
    const [fruit, setFruit] = React.useState('');
    const [on, setOn] = React.useState(false);
    const [checked, setChecked] = React.useState(false);
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
    );
  }
  return { Component };
}
