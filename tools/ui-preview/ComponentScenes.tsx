import { useState } from 'react';
import { resourceView, type ResourceReadState } from '../../src/resourceState';
import { Button, type ButtonVariant } from '../../src/ui/components/Button';
import { Card } from '../../src/ui/components/Card';
import { Input } from '../../src/ui/components/Input';
import { Modal } from '../../src/ui/components/Modal';
import {
  EmptyState,
  FilterBar,
  Notice,
  ResourceCard,
  ResourceDetail,
  ResourceLayout,
  ResourceList,
  SearchField,
  SectionHeader,
  StatusBadge,
} from '../../src/ui/components/PagePatterns';
import { Select } from '../../src/ui/components/Select';
import { Stack } from '../../src/ui/components/Stack';

type Locale = 'zh-CN' | 'en-US';
type SceneProps = { scenario: string; locale: Locale };

const LONG_PATH = '/Users/preview/Projects/DG-OS/workspaces/customer-research/2026-quarterly-review/knowledge-base/source-documents/architecture-and-deployment-notes.md';

const COPY = {
  'zh-CN': {
    controlsTitle: '生产控件预览',
    controlsDescription: '使用真实生产控件检查字号、密度和窄窗表现。所有输入与操作只保存在此预览中。',
    buttons: '按钮与操作反馈',
    primary: '主要操作', secondary: '次要操作', danger: '危险操作', ghost: '轻量操作', link: '链接操作',
    disabled: '不可用', loading: '处理中',
    clicked: '已在预览中执行：',
    fields: '输入与选择',
    uiLabel: 'UI 输入 · 工作区名称',
    readingLabel: '阅读输入 · 工作区说明',
    codeLabel: '代码输入 · 长路径',
    errorLabel: '错误输入 · 资源名称',
    error: '名称不能包含斜杠，请使用便于识别的名称。',
    name: '季度研究工作区',
    reading: '集中整理本季度的客户访谈、产品反馈和部署记录。阅读正文应保持舒适的行距，并在窄窗中自然换行。',
    selectLabel: '长选项选择 · 工作区策略',
    options: [
      '仅在当前工作区使用，保留本地草稿并在确认前检查所有资源变更',
      '在团队的所有研究工作区中使用，并保留完整的来源说明与版本记录',
      '只读预览，显示最近一次成功读取的资源及其完整路径，不执行任何写入',
      '针对包含中文、English 和长路径的混合内容使用独立的阅读与代码字号',
    ],
    openModal: '打开表单弹窗', modalTitle: '确认工作区预览设置',
    modalDescription: '这是一段较长的说明，用于检查真实弹窗在窄窗口和放大字号下的滚动、换行与键盘可达性。确认后只更新本页面的反馈，不会修改文件或工作区。请检查下面的名称、完整路径和说明，确保正文、表单以及底部按钮都可以通过键盘到达。',
    modalDetail: '最近一次成功读取的资料会保留在当前视图中，预览不会将这些资料发送到外部服务。长路径和混合语言内容用于呈现真实编辑场景。使用 Tab 在字段之间移动，或使用取消按钮与 Escape 关闭弹窗；再次打开时可以继续编辑当前预览内容。',
    modalName: '弹窗 · 工作区名称', modalPath: '弹窗 · 完整路径', modalNotes: '弹窗 · 阅读说明',
    confirm: '确认预览设置', cancel: '取消', confirmed: '已确认预览设置。更改仅保存在本页面。',
    patternsTitle: '资源页面模式',
    patternDescriptions: {
      'patterns-ready': '资源读取成功，列表和详情使用同一份确定的预览数据。',
      'patterns-initial-error': '首次读取失败，没有已确认的数据可显示。',
      'patterns-refresh-error': '刷新失败时保留最近一次成功读取的列表和详情。',
      'patterns-empty': '读取成功，但当前工作区没有资源。',
      'patterns-unconfirmed': '尚未读取资源，不将未确认状态显示为空列表。',
    },
    filterLabel: '资源筛选', searchLabel: '搜索预览资源', clear: '清除搜索',
    searchPlaceholder: '按名称或说明搜索', listLabel: '预览资源列表', detailLabel: '预览资源详情',
    refresh: '刷新预览', retry: '重试预览',
    retryFeedback: '已收到重试操作；此场景保留固定状态，便于重复检查。',
    emptyTitle: '工作区还没有资源', emptyDescription: '可以创建一个示例资源，查看本地操作反馈。',
    create: '创建示例资源', createFeedback: '已收到创建操作；此空状态预览不会写入资源。',
    noMatches: '没有匹配的预览资源', noMatchesDescription: '调整搜索词或清除搜索，重新显示示例资源。',
    selectResource: '选择一个资源', selectDescription: '从列表选择资源以查看完整说明和路径。',
    ready: '已确认', retained: '上次成功数据',
    firstError: '预览资源读取失败：连接暂时不可用。可使用重试按钮检查错误操作反馈。',
    refreshError: '预览刷新失败：连接暂时不可用。最近一次成功读取的数据仍然可用。',
    resourceName: '资源名称', resourcePath: '完整路径', resourceDescription: '资源说明',
    save: '保存预览备注', saved: '预览备注已保存在本页面。',
    resources: [
      { id: 'research', name: '季度客户研究与产品反馈汇总', description: '整理客户访谈、可访问性反馈和团队评审结论，保留每条记录的原始来源。' },
      { id: 'deployment', name: '部署流程与工作区维护说明', description: '记录本地工作区的准备步骤、运行约束，以及长路径资料的查阅方式。' },
    ],
  },
  'en-US': {
    controlsTitle: 'Production controls preview',
    controlsDescription: 'Inspect real production controls at different text sizes, densities, and window widths. Edits and actions stay in this preview.',
    buttons: 'Buttons and action feedback',
    primary: 'Primary action', secondary: 'Secondary action', danger: 'Danger action', ghost: 'Ghost action', link: 'Link action',
    disabled: 'Unavailable', loading: 'Processing',
    clicked: 'Executed in this preview: ',
    fields: 'Inputs and selection',
    uiLabel: 'UI input · Workspace name',
    readingLabel: 'Reading input · Workspace description',
    codeLabel: 'Code input · Long path',
    errorLabel: 'Error input · Resource name',
    error: 'The name cannot contain slashes. Choose a recognizable resource name.',
    name: 'Quarterly research workspace',
    reading: 'Collect customer interviews, product feedback, and deployment notes for this quarter. Reading content should retain comfortable line spacing and wrap naturally in a narrow window.',
    selectLabel: 'Long option select · Workspace policy',
    options: [
      'Use only in the current workspace, retain local drafts, and review every resource change before confirming',
      'Use across all research workspaces in the team and retain complete source descriptions and version history',
      'Read-only preview showing the last successfully loaded resources and their complete paths without writing changes',
      'Use separate reading and code text sizes for mixed content containing 中文, English, and long file paths',
    ],
    openModal: 'Open form dialog', modalTitle: 'Confirm workspace preview settings',
    modalDescription: 'This longer description exercises the production dialog’s scrolling, wrapping, and keyboard access in a narrow window and at enlarged text sizes. Confirming only updates feedback on this page; it does not modify files or workspaces. Review the name, complete path, and description below. The body, fields, and footer actions should all remain reachable with the keyboard.',
    modalDetail: 'The most recently loaded material remains visible in this view, and the preview does not send it to an external service. Long paths and mixed-language content represent a realistic editing session. Use Tab to move between fields, or Cancel and Escape to close the dialog. Reopening it lets you continue editing the current preview content.',
    modalName: 'Dialog · Workspace name', modalPath: 'Dialog · Complete path', modalNotes: 'Dialog · Reading notes',
    confirm: 'Confirm preview settings', cancel: 'Cancel', confirmed: 'Preview settings confirmed. Changes are stored on this page only.',
    patternsTitle: 'Resource page patterns',
    patternDescriptions: {
      'patterns-ready': 'Resources loaded successfully. The list and detail use the same deterministic preview data.',
      'patterns-initial-error': 'The first read failed. No confirmed resource data is available to display.',
      'patterns-refresh-error': 'A failed refresh retains the most recently loaded list and detail.',
      'patterns-empty': 'The read succeeded, but the current workspace contains no resources.',
      'patterns-unconfirmed': 'Resources have not been read. An unconfirmed state is not presented as an empty list.',
    },
    filterLabel: 'Resource filters', searchLabel: 'Search preview resources', clear: 'Clear search',
    searchPlaceholder: 'Search names or descriptions', listLabel: 'Preview resource list', detailLabel: 'Preview resource detail',
    refresh: 'Refresh preview', retry: 'Retry preview',
    retryFeedback: 'Retry received. This scene keeps its fixed state so it can be inspected repeatedly.',
    emptyTitle: 'No resources in this workspace', emptyDescription: 'Create an example resource to see local action feedback.',
    create: 'Create example resource', createFeedback: 'Create received. This empty-state preview does not write resources.',
    noMatches: 'No matching preview resources', noMatchesDescription: 'Change the search text or clear it to show the example resources again.',
    selectResource: 'Select a resource', selectDescription: 'Choose a resource from the list to inspect its complete description and path.',
    ready: 'Confirmed', retained: 'Last successful data',
    firstError: 'The preview resource read failed: the connection is temporarily unavailable. Retry to inspect local error-action feedback.',
    refreshError: 'The preview refresh failed: the connection is temporarily unavailable. The last successfully loaded data remains available.',
    resourceName: 'Resource name', resourcePath: 'Complete path', resourceDescription: 'Resource description',
    save: 'Save preview notes', saved: 'Preview notes saved on this page.',
    resources: [
      { id: 'research', name: 'Quarterly customer research and product feedback', description: 'Collect customer interviews, accessibility feedback, and team review decisions while retaining the original source of each record.' },
      { id: 'deployment', name: 'Deployment process and workspace maintenance', description: 'Document local workspace preparation, runtime constraints, and ways to browse documents with long file paths.' },
    ],
  },
};

function ControlsScene({ scenario, locale }: SceneProps) {
  const copy = COPY[locale];
  const [name, setName] = useState(copy.name);
  const [reading, setReading] = useState(copy.reading);
  const [path, setPath] = useState(LONG_PATH);
  const [invalidName, setInvalidName] = useState('research/quarterly');
  const [policy, setPolicy] = useState('policy-0');
  const [feedback, setFeedback] = useState('');
  const [modalOpen, setModalOpen] = useState(scenario === 'modal');
  const [modalName, setModalName] = useState(copy.name);
  const [modalPath, setModalPath] = useState(LONG_PATH);
  const [modalNotes, setModalNotes] = useState(copy.reading);
  const variants: ButtonVariant[] = ['primary', 'secondary', 'danger', 'ghost', 'link'];

  return (
    <main data-preview-controls className="min-w-0 p-4">
      <Stack gap={6}>
        <SectionHeader as="h1" title={copy.controlsTitle} description={copy.controlsDescription} actions={<Button data-preview-open-modal onClick={() => setModalOpen(true)}>{copy.openModal}</Button>} />
        <Card>
          <Card.Header><h2 className="text-base font-semibold">{copy.buttons}</h2></Card.Header>
          <Card.Body>
            <div className="flex min-w-0 flex-wrap items-center gap-3">
              {variants.map(variant => <Button key={variant} variant={variant} onClick={() => setFeedback(copy.clicked + copy[variant])}>{copy[variant]}</Button>)}
              <Button disabled variant="secondary">{copy.disabled}</Button>
              <Button loading variant="secondary">{copy.loading}</Button>
            </div>
          </Card.Body>
        </Card>
        <Card>
          <Card.Header><h2 className="text-base font-semibold">{copy.fields}</h2></Card.Header>
          <Card.Body>
            <Stack gap={4}>
              <div data-preview-input-domain="ui"><Input id="preview-input-ui" label={copy.uiLabel} value={name} onChange={event => setName(event.currentTarget.value)} /></div>
              <div data-preview-input-domain="reading"><Input id="preview-input-reading" multiline rows={3} className="type-reading" label={copy.readingLabel} value={reading} onChange={event => setReading(event.currentTarget.value)} /></div>
              <div data-preview-input-domain="code"><Input id="preview-input-code" multiline rows={2} className="type-code" label={copy.codeLabel} value={path} onChange={event => setPath(event.currentTarget.value)} /></div>
              <Input id="preview-input-error" label={copy.errorLabel} value={invalidName} error={invalidName.includes('/') ? copy.error : undefined} onChange={event => setInvalidName(event.currentTarget.value)} />
              <div data-preview-select="long" className="max-w-sm"><Select id="preview-select-long" label={copy.selectLabel} value={policy} options={copy.options.map((label, index) => ({ value: `policy-${index}`, label }))} onChange={setPolicy} /></div>
            </Stack>
          </Card.Body>
        </Card>
        {feedback ? <div data-preview-feedback><Notice tone="success">{feedback}</Notice></div> : null}
      </Stack>
      <Modal
        open={modalOpen}
        onOpenChange={setModalOpen}
        kind="form"
        title={copy.modalTitle}
        description={<Stack gap={3}><p className="type-reading">{copy.modalDescription}</p><p className="type-reading">{copy.modalDetail}</p></Stack>}
        confirmText={copy.confirm}
        cancelText={copy.cancel}
        onConfirm={() => setFeedback(copy.confirmed)}
      >
        <Input id="preview-modal-name" label={copy.modalName} value={modalName} onChange={event => setModalName(event.currentTarget.value)} />
        <Input id="preview-modal-path" multiline rows={3} className="type-code" label={copy.modalPath} value={modalPath} onChange={event => setModalPath(event.currentTarget.value)} />
        <Input id="preview-modal-notes" multiline rows={4} className="type-reading" label={copy.modalNotes} value={modalNotes} onChange={event => setModalNotes(event.currentTarget.value)} />
      </Modal>
    </main>
  );
}

function PatternsScene({ scenario, locale }: SceneProps) {
  const copy = COPY[locale];
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState('research');
  const [notes, setNotes] = useState(copy.reading);
  const [feedback, setFeedback] = useState('');
  const read: ResourceReadState = scenario === 'patterns-initial-error'
    ? { status: 'error', error: copy.firstError }
    : scenario === 'patterns-unconfirmed'
      ? { status: 'idle' }
      : { status: 'ready', refresh: scenario === 'patterns-refresh-error' ? { status: 'error', error: copy.refreshError } : { status: 'current' } };
  const resources = scenario === 'patterns-empty' ? [] : copy.resources;
  const query = search.trim().toLocaleLowerCase(locale);
  const filtered = resources.filter(resource => `${resource.name} ${resource.description}`.toLocaleLowerCase(locale).includes(query));
  const selected = filtered.find(resource => resource.id === selectedId);
  const retry = <Button variant="secondary" onClick={() => setFeedback(copy.retryFeedback)}>{copy.retry}</Button>;
  const empty = scenario === 'patterns-empty'
    ? <EmptyState title={copy.emptyTitle} description={copy.emptyDescription} action={<Button onClick={() => setFeedback(copy.createFeedback)}>{copy.create}</Button>} />
    : <EmptyState title={copy.noMatches} description={copy.noMatchesDescription} action={<Button variant="secondary" onClick={() => setSearch('')}>{copy.clear}</Button>} />;
  const description = copy.patternDescriptions[scenario as keyof typeof copy.patternDescriptions] ?? copy.patternDescriptions['patterns-ready'];

  return (
    <main data-preview-patterns data-preview-pattern-state={scenario} className="min-w-0 p-4">
      <Stack gap={4}>
        <SectionHeader as="h1" title={copy.patternsTitle} description={description} actions={<Button variant="secondary" onClick={() => setFeedback(copy.retryFeedback)}>{copy.refresh}</Button>} />
        <FilterBar label={copy.filterLabel} onSubmit={event => event.preventDefault()}>
          <SearchField className="min-w-0 flex-1" label={copy.searchLabel} placeholder={copy.searchPlaceholder} value={search} onChange={setSearch} onClear={() => setSearch('')} clearLabel={copy.clear} />
        </FilterBar>
        {feedback ? <div data-preview-feedback><Notice tone="success">{feedback}</Notice></div> : null}
        <ResourceLayout
          listLabel={copy.listLabel}
          detailLabel={copy.detailLabel}
          list={
            <ResourceList state={resourceView(read, filtered.length === 0)} label={copy.listLabel} retry={retry} emptyContent={empty}>
              {filtered.map(resource => <ResourceCard key={resource.id} title={resource.name} description={resource.description} className={selected?.id === resource.id ? 'border-accent-default bg-surface-selected' : undefined} onSelect={() => setSelectedId(resource.id)} status={<StatusBadge status={scenario === 'patterns-refresh-error' ? 'warning' : 'success'} label={scenario === 'patterns-refresh-error' ? copy.retained : copy.ready} />} />)}
            </ResourceList>
          }
          detail={
            <ResourceDetail state={resourceView(read, !selected)} label={copy.detailLabel} retry={retry} readOnly={scenario === 'patterns-refresh-error'} emptyContent={<EmptyState title={copy.selectResource} description={copy.selectDescription} />}>
              {selected ? <Stack gap={4}>
                <SectionHeader as="h2" title={selected.name} description={selected.description} />
                <Input label={copy.resourceName} value={selected.name} readOnly />
                <Input multiline rows={3} className="type-code" label={copy.resourcePath} value={LONG_PATH} readOnly />
                <Input multiline rows={4} className="type-reading" label={copy.resourceDescription} value={notes} readOnly={scenario === 'patterns-refresh-error'} onChange={event => setNotes(event.currentTarget.value)} />
                <div className="flex flex-wrap gap-2"><Button disabled={scenario === 'patterns-refresh-error'} onClick={() => setFeedback(copy.saved)}>{copy.save}</Button></div>
              </Stack> : null}
            </ResourceDetail>
          }
        />
      </Stack>
    </main>
  );
}

export default function ComponentScenes({ scenario, locale }: SceneProps) {
  return scenario.startsWith('patterns-')
    ? <PatternsScene key={`${scenario}:${locale}`} scenario={scenario} locale={locale} />
    : <ControlsScene key={`${scenario}:${locale}`} scenario={scenario} locale={locale} />;
}
