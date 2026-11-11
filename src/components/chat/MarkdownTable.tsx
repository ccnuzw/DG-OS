import { useId, useMemo, useState, type ReactNode } from 'react';
import type { Tokens } from 'marked';
import { Button, cn } from '@dgos/ui';
import { ArrowDown, ArrowUp, ArrowUpDown, ChevronDown, ChevronLeft, ChevronRight, Copy, Download, RotateCcw, Table2 } from 'lucide-react';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { t, useLocale } from '../../i18n';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import { tableData } from './richMarkdownSafety';
import { RichMarkdownBoundary } from './RichMarkdown';

const ROWS_PER_PAGE = 50;
const COLUMNS_PER_PAGE = 12;
const ALIGN_CLASS: Readonly<Record<string, string>> = { left: 'text-left', center: 'text-center', right: 'text-right' };

export function MarkdownTable({ table, renderCell }: { table: Tokens.Table; renderCell: (cell: Tokens.TableCell) => ReactNode }) {
  const { locale } = useLocale();
  const [expanded, setExpanded] = useState(false);
  const [page, setPage] = useState(0);
  const [columnPage, setColumnPage] = useState(0);
  const [sort, setSort] = useState<{ column: number; direction: 1 | -1 } | null>(null);
  const [copyState, copy] = useCopyFeedback();
  const [exportState, setExportState] = useState<'exported' | 'exportFailed' | null>(null);
  const tableId = useId();
  const large = table.rows.length > 30 || table.header.length > 8 || table.rows.length * table.header.length > 180;
  const collator = useMemo(() => new Intl.Collator(locale, { numeric: true, sensitivity: 'base' }), [locale]);
  const rows = useMemo(() => {
    if (!expanded || !sort) return table.rows;
    return table.rows.map((cells, index) => ({ cells, index })).sort((a, b) => sort.direction * collator.compare(a.cells[sort.column]?.text ?? '', b.cells[sort.column]?.text ?? '') || a.index - b.index).map(row => row.cells);
  }, [table.rows, expanded, sort, collator]);
  const rowStart = Math.min(page * ROWS_PER_PAGE, Math.max(0, Math.ceil(rows.length / ROWS_PER_PAGE) - 1) * ROWS_PER_PAGE);
  const columnStart = Math.min(columnPage * COLUMNS_PER_PAGE, Math.max(0, Math.ceil(table.header.length / COLUMNS_PER_PAGE) - 1) * COLUMNS_PER_PAGE);
  const columns = large ? table.header.slice(columnStart, columnStart + COLUMNS_PER_PAGE) : table.header;
  const shownRows = large ? rows.slice(rowStart, rowStart + ROWS_PER_PAGE) : rows;
  const completeData = (delimiter: ',' | '\t') => tableData([table.header.map(cell => cell.text), ...table.rows.map(row => row.map(cell => cell.text))], delimiter);
  const exportCsv = () => {
    let url: string | undefined;
    const link = document.createElement('a');
    try {
      url = URL.createObjectURL(new Blob(['\ufeff', completeData(',')], { type: 'text/csv;charset=utf-8' }));
      link.href = url;
      link.download = 'table.csv';
      document.body.appendChild(link);
      link.click();
      setExportState('exported');
    } catch { setExportState('exportFailed'); }
    finally {
      link.remove();
      if (url) { const completedUrl = url; const revoke = URL.revokeObjectURL.bind(URL); window.setTimeout(() => revoke(completedUrl), 1000); }
    }
  };
  const cellClass = (cell: Tokens.TableCell, header: boolean) => cn('border border-border-subtle px-2 py-1', header && 'bg-surface-inset font-semibold', cell.align ? ALIGN_CLASS[cell.align] : 'text-left');
  return <div className={cn('chat-table', large && 'chat-table-large')}>
    <div className={cn('chat-table-toolbar', !large && 'chat-table-toolbar-small')}>
      <span className="chat-table-heading"><span className="chat-table-icon"><Table2 size={15} aria-hidden="true" /></span><strong>{t(large ? 'richMarkdown.largeTable' : 'richMarkdown.table', { rows: table.rows.length, columns: table.header.length })}</strong>{!large ? <small>{t('richMarkdown.tableSize', { rows: table.rows.length, columns: table.header.length })}</small> : null}</span>
      {large ? <span className="chat-table-actions">
        <Button size="xs" variant="secondary" aria-expanded={expanded} aria-controls={tableId} onClick={() => setExpanded(value => !value)}><ChevronDown size={13} className={cn('chat-rich-disclosure-chevron', expanded && 'chat-rich-disclosure-open')} aria-hidden="true" />{t(expanded ? 'richMarkdown.hideTable' : 'richMarkdown.showTable')}</Button>
        <Button size="xs" variant="ghost" onClick={() => copy(completeData('\t'))}><Copy size={13} aria-hidden="true" />{t('richMarkdown.copyTable')}</Button>
        <Button size="xs" variant="ghost" onClick={exportCsv}><Download size={13} aria-hidden="true" />{t('richMarkdown.exportCsv')}</Button>
      </span> : null}
      <span role="status" className={cn('chat-table-feedback', copyState === 'failed' || exportState === 'exportFailed' ? 'text-status-error-text' : 'text-status-success-text')}>{COPY_TEXT[copyState]}{exportState ? ` ${t(`richMarkdown.${exportState}`)}` : ''}</span>
    </div>
    {!large || expanded ? <>
      {large ? <div className="chat-table-pagination">
        <span role="status">{t('richMarkdown.tableWindow', { start: rows.length ? rowStart + 1 : 0, end: Math.min(rowStart + ROWS_PER_PAGE, rows.length), total: rows.length, firstColumn: columnStart + 1, lastColumn: Math.min(columnStart + COLUMNS_PER_PAGE, table.header.length), columns: table.header.length })}</span>
        <span className="chat-table-page-actions">
          <Button size="xs" variant="ghost" disabled={!sort} onClick={() => { setSort(null); setPage(0); }}><RotateCcw size={13} aria-hidden="true" />{t('richMarkdown.originalOrder')}</Button>
          <Button size="xs" variant="ghost" disabled={rowStart === 0} onClick={() => setPage(value => Math.max(0, value - 1))}><ChevronLeft size={13} aria-hidden="true" />{t('richMarkdown.previousRows')}</Button>
          <Button size="xs" variant="ghost" disabled={rowStart + ROWS_PER_PAGE >= rows.length} onClick={() => setPage(value => value + 1)}>{t('richMarkdown.nextRows')}<ChevronRight size={13} aria-hidden="true" /></Button>
          {table.header.length > COLUMNS_PER_PAGE ? <><Button size="xs" variant="ghost" disabled={columnStart === 0} onClick={() => setColumnPage(value => Math.max(0, value - 1))}><ChevronLeft size={13} aria-hidden="true" />{t('richMarkdown.previousColumns')}</Button><Button size="xs" variant="ghost" disabled={columnStart + COLUMNS_PER_PAGE >= table.header.length} onClick={() => setColumnPage(value => value + 1)}>{t('richMarkdown.nextColumns')}<ChevronRight size={13} aria-hidden="true" /></Button></> : null}
        </span>
      </div> : null}
      <div id={tableId} className={cn('chat-table-scroll', FOCUS_RING)} tabIndex={0} role="region" aria-label={t('richMarkdown.table')}>
        <table className="border-collapse"><thead><tr>{columns.map((cell, index) => {
          const column = (large ? columnStart : 0) + index;
          return <th key={column} scope="col" className={cellClass(cell, true)} aria-sort={large ? sort?.column === column ? sort.direction === 1 ? 'ascending' : 'descending' : 'none' : undefined}>
            {large ? <Button className="chat-table-sort" size="xs" variant="ghost" aria-label={t('richMarkdown.sortColumn', { column: cell.text })} onClick={() => { setSort(previous => ({ column, direction: previous?.column === column && previous.direction === 1 ? -1 : 1 })); setPage(0); }}><span>{cell.text}</span>{sort?.column === column ? sort.direction === 1 ? <ArrowUp size={13} aria-hidden="true" /> : <ArrowDown size={13} aria-hidden="true" /> : <ArrowUpDown className="chat-table-sort-idle" size={13} aria-hidden="true" />}</Button> : renderCell(cell)}
          </th>;
        })}</tr></thead><tbody>{shownRows.map((row, index) => <tr key={rowStart + index}>{(large ? row.slice(columnStart, columnStart + COLUMNS_PER_PAGE) : row).map((cell, index) => <td key={index} className={cellClass(cell, false)}>{renderCell(cell)}</td>)}</tr>)}</tbody></table>
      </div>
    </> : null}
    {large ? <RichMarkdownBoundary kind="table" /> : null}
  </div>;
}
