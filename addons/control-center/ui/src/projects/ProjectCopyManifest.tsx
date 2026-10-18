import { useMemo, useState } from 'react';
import type { CopyJob } from './ProjectCopyPanel';

/** Keep large snapshots searchable without rendering thousands of rows at once. */
export function ProjectCopyManifest({ job, language, formatSize }: { job: CopyJob; language: string; formatSize: (bytes: number) => string }) {
  const t = (cn: string, en: string) => language.startsWith('zh') ? cn : en;
  const [query, setQuery] = useState(''), [limit, setLimit] = useState(200), [skipLimit, setSkipLimit] = useState(200);
  const entries = useMemo(() => [...job.summary.files.map(file => ({ path: file.path, bytes: file.size })), ...job.summary.directories.map(path => ({ path: path + '/', bytes: undefined }))], [job.summary.files, job.summary.directories]);
  const filtered = useMemo(() => entries.filter(entry => entry.path.toLowerCase().includes(query.trim().toLowerCase())), [entries, query]);
  return <>
    <details><summary>{t('检查文件清单', 'Review file manifest')}</summary>
      {entries.length > 200 && <label>{t('按路径筛选', 'Filter paths')}<input type="search" value={query} onChange={event => { setQuery(event.target.value); setLimit(200); }} /></label>}
      <ul>{filtered.slice(0, limit).map(entry => <li key={entry.path}><code>{entry.path}</code>{entry.bytes !== undefined && <span>{formatSize(entry.bytes)}</span>}</li>)}</ul>
      {!filtered.length && <p>{t('没有匹配的路径', 'No matching paths')}</p>}
      {filtered.length > limit && <button type="button" onClick={() => setLimit(value => value + 200)}>{t('显示更多路径', 'Show more paths')} ({limit} / {filtered.length})</button>}
    </details>
    <details><summary>{t('已跳过 ', 'Skipped entries: ') + job.summary.skipped.length}</summary>
      <ul>{job.summary.skipped.slice(0, skipLimit).map(path => <li key={path}><code>{path}</code></li>)}</ul>
      {job.summary.skipped.length > skipLimit && <button type="button" onClick={() => setSkipLimit(value => value + 200)}>{t('显示更多跳过项', 'Show more skipped entries')} ({skipLimit} / {job.summary.skipped.length})</button>}
    </details>
  </>;
}
