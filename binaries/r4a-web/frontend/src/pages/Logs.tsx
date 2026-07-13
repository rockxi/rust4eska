import React, { useState, useEffect, useRef } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ScrollText, RefreshCw, Radio } from 'lucide-react';
import apiClient from '../api/client';

interface LogEntry {
    node: string;
    container: string;
    ts_ms: number;
    stream: string;
    line: string;
}

const MAX_LINES = 2000;

// [node, container] пары
const fetchLogContainers = async (): Promise<[string, string][]> => {
    const r = await apiClient.get('/logs/containers');
    return r.data;
};

const fetchLogs = async (node: string, container: string, tail: number): Promise<LogEntry[]> => {
    const r = await apiClient.get(
        `/logs?node=${encodeURIComponent(node)}&container=${encodeURIComponent(container)}&tail=${tail}`
    );
    return r.data;
};

function lineColor(entry: LogEntry) {
    const l = entry.line.toLowerCase();
    if (entry.stream === 'stderr' || l.includes('error') || l.includes('err]')) return 'text-red-400';
    if (l.includes('warn')) return 'text-yellow-400';
    return 'text-gray-300';
}

function formatTs(ts_ms: number) {
    const d = new Date(ts_ms);
    return d.toLocaleTimeString('en-GB', { hour12: false }) + '.' + String(ts_ms % 1000).padStart(3, '0');
}

const Logs: React.FC = () => {
    const [selected, setSelected] = useState<{ node: string; container: string } | null>(null);
    const [tail, setTail] = useState(200);
    const [live, setLive] = useState(true);
    const [entries, setEntries] = useState<LogEntry[]>([]);
    const [historyLoading, setHistoryLoading] = useState(false);
    const logRef = useRef<HTMLDivElement>(null);

    const { data: containers, isLoading } = useQuery({
        queryKey: ['log-containers'],
        queryFn: fetchLogContainers,
        refetchInterval: 15000,
    });

    // Автовыбор первого контейнера
    useEffect(() => {
        if (!selected && containers && containers.length > 0) {
            setSelected({ node: containers[0][0], container: containers[0][1] });
        }
    }, [containers, selected]);

    // История при выборе контейнера / смене tail
    useEffect(() => {
        if (!selected) return;
        let cancelled = false;
        setHistoryLoading(true);
        fetchLogs(selected.node, selected.container, tail)
            .then(lines => { if (!cancelled) setEntries(lines); })
            .catch(() => { if (!cancelled) setEntries([]); })
            .finally(() => { if (!cancelled) setHistoryLoading(false); });
        return () => { cancelled = true; };
    }, [selected, tail]);

    // Live-стрим через SSE (EventSource не умеет заголовки — токен в query)
    useEffect(() => {
        if (!selected || !live) return;
        const token = sessionStorage.getItem('r4a_token') || '';
        const url = `${apiClient.defaults.baseURL}/logs/stream?node=${encodeURIComponent(selected.node)}&container=${encodeURIComponent(selected.container)}&token=${encodeURIComponent(token)}`;
        const es = new EventSource(url);
        es.onmessage = ev => {
            try {
                const entry: LogEntry = JSON.parse(ev.data);
                setEntries(prev => {
                    const next = [...prev, entry];
                    return next.length > MAX_LINES ? next.slice(next.length - MAX_LINES) : next;
                });
            } catch { /* пропускаем битые события */ }
        };
        return () => es.close();
    }, [selected, live]);

    // Автоскролл вниз
    useEffect(() => {
        if (logRef.current) {
            logRef.current.scrollTop = logRef.current.scrollHeight;
        }
    }, [entries]);

    const refetchHistory = () => {
        if (!selected) return;
        setHistoryLoading(true);
        fetchLogs(selected.node, selected.container, tail)
            .then(setEntries)
            .catch(() => setEntries([]))
            .finally(() => setHistoryLoading(false));
    };

    const selectedKey = selected ? `${selected.node}/${selected.container}` : '';

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="mb-6">
                <h1 className="text-3xl font-bold text-white tracking-tight">Logs</h1>
                <p className="text-text-silver mt-2">Centralized container logs from all nodes</p>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal" />
                </div>
            ) : !containers || containers.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <ScrollText className="w-12 h-12 text-gray-600 mx-auto mb-4" />
                    <p className="text-gray-400 text-lg">No container logs collected yet</p>
                </div>
            ) : (
                <div className="bg-slate-dark border border-gray-800 rounded-lg flex flex-col overflow-hidden">
                    <div className="flex items-center gap-3 px-5 py-4 border-b border-gray-800/50 shrink-0 flex-wrap">
                        <ScrollText className="w-4 h-4 text-accent-teal" />
                        <select
                            value={selectedKey}
                            onChange={e => {
                                const [node, ...rest] = e.target.value.split('/');
                                setSelected({ node, container: rest.join('/') });
                            }}
                            className="bg-deep-dark border border-gray-700 text-white text-sm rounded px-2 py-1 focus:outline-none font-mono"
                        >
                            {containers.map(([node, container]) => (
                                <option key={`${node}/${container}`} value={`${node}/${container}`}>
                                    {container} @ {node}
                                </option>
                            ))}
                        </select>
                        <div className="ml-auto flex items-center gap-3">
                            <select
                                value={tail}
                                onChange={e => setTail(Number(e.target.value))}
                                className="bg-deep-dark border border-gray-700 text-white text-sm rounded px-2 py-1 focus:outline-none"
                            >
                                <option value={50}>Last 50 lines</option>
                                <option value={200}>Last 200 lines</option>
                                <option value={500}>Last 500 lines</option>
                                <option value={1000}>Last 1000 lines</option>
                            </select>
                            <button
                                onClick={() => setLive(l => !l)}
                                className={`flex items-center gap-1.5 px-3 py-1.5 rounded text-xs transition-colors ${
                                    live
                                        ? 'bg-green-900/50 text-green-400'
                                        : 'bg-gray-800 text-gray-400 hover:text-white'
                                }`}
                                title={live ? 'Live stream on' : 'Live stream off'}
                            >
                                <Radio className={`w-3.5 h-3.5 ${live ? 'animate-pulse' : ''}`} />
                                Live
                            </button>
                            <button
                                onClick={refetchHistory}
                                className="p-1.5 text-gray-400 hover:text-white hover:bg-gray-800 rounded transition-colors"
                                title="Reload history"
                            >
                                <RefreshCw className={`w-4 h-4 ${historyLoading ? 'animate-spin' : ''}`} />
                            </button>
                        </div>
                    </div>

                    <div
                        ref={logRef}
                        className="h-[70vh] overflow-y-auto p-4 font-mono text-xs leading-5 bg-deep-dark"
                    >
                        {historyLoading && entries.length === 0 ? (
                            <div className="flex items-center justify-center h-full">
                                <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-accent-teal" />
                            </div>
                        ) : entries.length === 0 ? (
                            <p className="text-gray-500 italic">No logs</p>
                        ) : (
                            entries.map((e, i) => (
                                <div key={i} className={`whitespace-pre-wrap break-all ${lineColor(e)}`}>
                                    <span className="text-gray-600 select-none">{formatTs(e.ts_ms)} </span>
                                    {e.line || ' '}
                                </div>
                            ))
                        )}
                    </div>
                </div>
            )}
        </div>
    );
};

export default Logs;
