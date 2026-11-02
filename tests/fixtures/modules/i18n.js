export default function createModule(host) {
  return {
    Component() {
      const [locale, setLocale] = host.React.useState(host.i18n.locale);
      host.React.useEffect(() => host.i18n.subscribe(setLocale), []);
      return host.React.createElement('span', { 'data-testid': 'module-locale' }, locale);
    },
  };
}
