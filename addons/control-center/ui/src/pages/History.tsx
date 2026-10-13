import { RefreshCw } from 'lucide-react';
import { useEffect,useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { Button,Card,Empty,Filter,PageHead,Row,Status } from '../components';
import type { Operation } from '../types';
import { request,t,useWorkspace } from '../workspace';

const when = (timestamp: number) => new Date(timestamp < 1e12 ? timestamp * 1000 : timestamp).toLocaleString();
export function History() {
  const { state, snapshot } = useWorkspace();
  const [params, setParams] = useSearchParams();
  const kind = params.get('kind') === 'remote' ? 'remote' : 'local', id = params.get('id') || '';
  const [records, setRecords] = useState<Operation[]>([]), [selected, setSelected] = useState<Operation | null>(null);
  const [error, setError] = useState(''), [search, setSearch] = useState(''), [loading, setLoading] = useState(false), [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true); setError('');
    if (kind === 'remote' && !snapshot?.remote_nodes?.available) { setRecords([]); setLoading(false); return; }
    void request(kind === 'local' ? '/api/history/list' : '/api/nodes/jobs', {}).then(value => { if (active) setRecords(value.records || value.jobs); }).catch(problem => { if (active) setError(problem.message); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [kind, state.snapshotAt, state.busy, revision, snapshot?.remote_nodes?.available]);
  useEffect(() => {
    let active = true, timer: ReturnType<typeof setTimeout>;
    setSelected(null);
    let output = '', stdout = '', stderr = '';
    let outputCursor = 0, stdoutCursor = 0, stderrCursor = 0;
    const load = async () => {
      if (!id) return;
      try {
        const value = await request(kind === 'local' ? '/api/history/get' : '/api/nodes/job', { id, outputCursor, stdoutCursor, stderrCursor });
        if (!active) return;
        output = value.outputReset ? value.outputDelta : output + (value.outputDelta || '');
        stdout = value.stdoutReset ? value.stdoutDelta : stdout + (value.stdoutDelta || '');
        stderr = value.stderrReset ? value.stderrDelta : stderr + (value.stderrDelta || '');
        outputCursor = value.outputCursor; stdoutCursor = value.stdoutCursor; stderrCursor = value.stderrCursor;
        output = output.slice(-128 * 1024);
        setSelected({ ...value, output, stdout, stderr }); setError('');
        if (value.outcome === 'running') timer = setTimeout(() => void load(), 1000);
      } catch (problem) { if (active) setError(problem instanceof Error ? problem.message : String(problem)); }
    };
    void load();
    return () => { active = false; clearTimeout(timer); };
  }, [kind, id, revision]);
  const filtered = records.filter(record => [record.title, record.node, record.program, record.outcome].join(' ').toLowerCase().includes(search.toLowerCase()));
  return <><PageHead title={t('操作历史', 'Operation history')} description={t('保留最近 128 条记录。服务重启后未完成任务标记为中断，不自动重试。', 'Keep the latest 128 records. Unfinished tasks are marked interrupted after restart and are never replayed.')} action={<Button local icon={RefreshCw} disabled={loading} onClick={() => setRevision(value => value + 1)}>{t('刷新', 'Refresh')}</Button>} />
    <div className="segmented" role="group" aria-label={t('历史类型', 'History type')}>{(['local', 'remote'] as const).map(value => <Button local key={value} tone={kind === value ? 'primary' : undefined} onClick={() => { setParams({ kind: value }); setError(''); }}>{value === 'local' ? t('工作区操作', 'Workspace operations') : t('节点任务', 'Node tasks')}</Button>)}</div>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="grid two"><Card title={t('任务记录', 'Records')}><Filter value={search} onChange={setSearch} label={t('搜索记录', 'Search records')} />{filtered.map(record => <Row key={record.id} title={record.title || [record.node, record.program].filter(Boolean).join(' · ') || record.id} detail={when(record.created_at)}><div className="actions"><Status state={record.outcome === 'exited' && record.exit_code != null && record.exit_code !== 0 ? 'failed' : record.outcome} /><Button local onClick={() => setParams({ kind, id: record.id })} aria-pressed={record.id === id}>{t('详情', 'Details')}</Button></div></Row>)}{!filtered.length && <Empty title={loading ? t('正在加载…', 'Loading…') : t('暂无记录', 'No records yet')} />}</Card>
    <Card title={t('任务详情', 'Operation details')}>{selected ? <><div className="actions"><Status state={selected.outcome === 'exited' && selected.exit_code != null && selected.exit_code !== 0 ? 'failed' : selected.outcome} />{selected.exit_code !== undefined && selected.exit_code !== null && <span>{t('退出码 ', 'Exit code ')}{selected.exit_code}</span>}{kind === 'remote' && selected.outcome === 'running' && <Button local tone="danger-ghost" onClick={() => { void request('/api/nodes/cancel', { id }).then(() => setRevision(value => value + 1)).catch(problem => setError(problem.message)); }}>{t('取消任务', 'Cancel task')}</Button>}</div><p className="muted">{selected.id}</p>{selected.program && <code className="code-block">{selected.program} {JSON.stringify(selected.args || [])}</code>}{selected.error && <p className="form-error">{selected.error}</p>}{selected.truncated && <p className="muted">{t('输出超过保留上限，部分内容已截断。', 'Output exceeded the retention limit and was truncated.')}</p>}<pre className="history-output">{kind === 'local' ? selected.output || t('暂无输出。', 'No output yet.') : (selected.stdout || '') + (selected.stderr ? '\n[stderr]\n' + selected.stderr : '') || t('暂无输出。', 'No output yet.')}</pre></> : <Empty title={id ? t('正在读取任务…', 'Loading operation…') : t('选择一条记录', 'Select an operation')} />}</Card></div></>;
}
