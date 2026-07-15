import React, { useState, useEffect, useRef } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Database, RefreshCw, Radio, ScrollText, Search, X } from 'lucide-react';
import apiClient from '../api/client';

interface LogEntry {
    node: string;
    container: string;
    ts_ms: number;
    stream: string;
    line: string;
}

interface LogsConfig {
    configured: boolean;
    node: string | null;
    endpoint: string | null;
    ready: boolean;
}

interface NodeInfo {
    ip: string;
    name: string;
    role: string;
}

const MAX_LINES = 2000;

const fetchLogsConfig = async (): Promise<LogsConfig> => {
    const r = await apiClient.get('/logs/config');
    return r.data;
};

const fetchNodes = async (): Promise<NodeInfo[]> => {
    const r = await apiClient.get('/nodes');
    return r.data;
};

// [node, container] пары
const fetchLogContainers = async (): Promise<[string, string][]> => {
    const r = await apiClient.get('/logs/containers');
    return r.data;
};

const fetchLogs = async (
    node: string,
    container: string,
    tail: number,
    opts?: { q?: string; stream?: string }
): Promise<LogEntry[]> => {
    const params = new URLSearchParams({ node, container, tail: String(tail) });
    if (opts?.q) params.set('q', opts.q);
    if (opts?.stream && opts.stream !== 'all') params.set('stream', opts.stream);
    const r = await apiClient.get(`/logs?${params.toString()}`);
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

type StreamFilter = 'all' | 'stdout' | 'stderr';

function highlightLine(line: string, query: string) {
    if (!query) return line;
    const parts = line.split(new RegExp(`(${query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')})`, 'ig'));
    return parts.map((part, i) =>
        part.toLowerCase() === query.toLowerCase() ? (
            <mark key={i} className="bg-yellow-500/40 text-yellow-100 rounded-sm">
                {part}
            </mark>
        ) : (
            part
        )
    );
}

const Logs: React.FC = () => {
    const [selected, setSelected] = useState<{ node: string; container: string } | null>(null);
    const [tail, setTail] = useState(200);
    const [live, setLive] = useState(true);
    const [entries, setEntries] = useState<LogEntry[]>([]);
    const [historyLoading, setHistoryLoading] = useState(false);
    const [search, setSearch] = useState('');
    const [debouncedSearch, setDebouncedSearch] = useState('');
    const [streamFilter, setStreamFilter] = useState<StreamFilter>('all');
    const [setupNode, setSetupNode] = useState('');
    const [setupEndpoint, setSetupEndpoint] = useState('');
    const [setupPending, setSetupPending] = useState(false);
    const [setupError, setSetupError] = useState<string | null>(null);
    const logRef = useRef<HTMLDivElement>(null);

    const { data: config, isLoading: configLoading, refetch: refetchConfig } = useQuery({
        queryKey: ['logs-config'],
        queryFn: fetchLogsConfig,
        refetchInterval: q => {
            const data = q.state.data;
            return data?.configured && !data.ready ? 2000 : false;
        },
    });

    const { data: nodes } = useQuery({
        queryKey: ['nodes'],
        queryFn: fetchNodes,
        enabled: !!config && !config.ready,
    });

    const { data: containers, isLoading } = useQuery({
        queryKey: ['log-containers'],
        queryFn: fetchLogContainers,
        refetchInterval: 15000,
        enabled: !!config?.ready,
    });

    const deployNodes = nodes || [];

    useEffect(() => {
        if (!setupNode && deployNodes.length > 0) {
            setSetupNode(deployNodes[0].name);
        }
    }, [deployNodes, setupNode]);

    // Автовыбор первого контейнера
    useEffect(() => {
        if (!selected && containers && containers.length > 0) {
            setSelected({ node: containers[0][0], container: containers[0][1] });
        }
    }, [containers, selected]);

    // Debounce поиска, чтобы не долбить ClickHouse на каждое нажатие клавиши
    useEffect(() => {
        const id = window.setTimeout(() => setDebouncedSearch(search.trim()), 300);
        return () => window.clearTimeout(id);
    }, [search]);

    // История при выборе контейнера / смене tail / фильтров — поиск и фильтр по stream
    // выполняются на стороне ClickHouse (WHERE + skip-индекс), не в браузере.
    useEffect(() => {
        if (!selected) return;
        let cancelled = false;
        setHistoryLoading(true);
        fetchLogs(selected.node, selected.container, tail, { q: debouncedSearch, stream: streamFilter })
            .then(lines => { if (!cancelled) setEntries(lines); })
            .catch(() => { if (!cancelled) setEntries([]); })
            .finally(() => { if (!cancelled) setHistoryLoading(false); });
        return () => { cancelled = true; };
    }, [selected, tail, debouncedSearch, streamFilter]);

    // Live через polling: ClickHouse является source of truth, SSE больше нет.
    useEffect(() => {
        if (!selected || !live) return;
        let cancelled = false;
        const tick = () => {
            fetchLogs(selected.node, selected.container, Math.min(MAX_LINES, Math.max(tail, 500)), {
                q: debouncedSearch,
                stream: streamFilter,
            })
                .then(lines => {
                    if (!cancelled) {
                        setEntries(lines.length > MAX_LINES ? lines.slice(lines.length - MAX_LINES) : lines);
                    }
                })
                .catch(() => {});
        };
        const id = window.setInterval(tick, 2000);
        return () => {
            cancelled = true;
            window.clearInterval(id);
        };
    }, [selected, live, debouncedSearch, streamFilter]);

    // Автоскролл вниз
    useEffect(() => {
        if (logRef.current) {
            logRef.current.scrollTop = logRef.current.scrollHeight;
        }
    }, [entries]);

    const refetchHistory = () => {
        if (!selected) return;
        setHistoryLoading(true);
        fetchLogs(selected.node, selected.container, tail, { q: debouncedSearch, stream: streamFilter })
            .then(setEntries)
            .catch(() => setEntries([]))
            .finally(() => setHistoryLoading(false));
    };

    const selectedKey = selected ? `${selected.node}/${selected.container}` : '';

    const deployClickHouse = async () => {
        if (!setupNode || setupPending) return;
        setSetupPending(true);
        setSetupError(null);
        try {
            await apiClient.post('/logs/setup', {
                node: setupNode,
                endpoint: setupEndpoint.trim() || null,
            });
            await refetchConfig();
        } catch (e: any) {
            setSetupError(e?.response?.data || e?.message || 'Failed to start ClickHouse deployment');
        } finally {
            setSetupPending(false);
        }
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="mb-6">
                <h1 className="text-3xl font-bold text-white tracking-tight">Logs</h1>
                <p className="text-text-silver mt-2">Centralized container logs from all nodes</p>
            </div>

            {configLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal" />
                </div>
            ) : !config?.ready ? (
                <div className="bg-slate-dark border border-gray-800 rounded-xl p-8 max-w-2xl">
                    <div className="flex items-start gap-4">
                        <div className="p-3 rounded-lg bg-accent-teal/10 border border-accent-teal/20">
                            <Database className="w-7 h-7 text-accent-teal" />
                        </div>
                        <div className="flex-1">
                            <h2 className="text-xl font-semibold text-white">
                                {config?.configured ? 'ClickHouse is starting' : 'Deploy ClickHouse for logs'}
                            </h2>
                            <p className="text-gray-400 mt-2 text-sm leading-6">
                                Logs are optional. r4a will deploy a managed ClickHouse container on the selected node,
                                create the schema, and agents will start shipping container logs directly to it.
                            </p>

                            {config?.configured ? (
                                <div className="mt-6 rounded-lg bg-deep-dark border border-gray-800 p-4">
                                    <p className="text-sm text-gray-300">
                                        Waiting for ClickHouse on <span className="text-white font-mono">{config.node}</span>
                                    </p>
                                    <p className="text-xs text-gray-500 font-mono mt-1">{config.endpoint}</p>
                                    <div className="mt-4 flex items-center gap-3 text-accent-teal text-sm">
                                        <RefreshCw className="w-4 h-4 animate-spin" />
                                        Initializing schema...
                                    </div>
                                </div>
                            ) : (
                                <div className="mt-6 space-y-4">
                                    <label className="block">
                                        <span className="block text-sm text-gray-300 mb-2">Node</span>
                                        <select
                                            value={setupNode}
                                            onChange={e => setSetupNode(e.target.value)}
                                            className="w-full bg-deep-dark border border-gray-700 text-white rounded px-3 py-2 focus:outline-none"
                                        >
                                            {deployNodes.map(node => (
                                                <option key={node.name} value={node.name}>
                                                    {node.name} ({node.role}, {node.ip})
                                                </option>
                                            ))}
                                        </select>
                                    </label>

                                    <label className="block">
                                        <span className="block text-sm text-gray-300 mb-2">Endpoint override (optional)</span>
                                        <input
                                            value={setupEndpoint}
                                            onChange={e => setSetupEndpoint(e.target.value)}
                                            placeholder="http://host.docker.internal:8123"
                                            className="w-full bg-deep-dark border border-gray-700 text-white rounded px-3 py-2 focus:outline-none font-mono text-sm"
                                        />
                                        <p className="text-xs text-gray-500 mt-2">
                                            Leave empty in production. Use an override for local Docker dev if the node VPN IP cannot reach the published port.
                                        </p>
                                    </label>

                                    {setupError && (
                                        <div className="text-sm text-red-400 bg-red-950/30 border border-red-900 rounded p-3">
                                            {setupError}
                                        </div>
                                    )}

                                    <button
                                        onClick={deployClickHouse}
                                        disabled={!setupNode || setupPending || deployNodes.length === 0}
                                        className="px-4 py-2 rounded bg-accent-teal text-deep-dark font-semibold disabled:opacity-50 disabled:cursor-not-allowed"
                                    >
                                        {setupPending ? 'Deploying...' : 'Deploy ClickHouse'}
                                    </button>
                                </div>
                            )}
                        </div>
                    </div>
                </div>
            ) : isLoading ? (
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
                        <div className="relative">
                            <Search className="w-3.5 h-3.5 text-gray-500 absolute left-2.5 top-1/2 -translate-y-1/2" />
                            <input
                                value={search}
                                onChange={e => setSearch(e.target.value)}
                                placeholder="Search logs..."
                                className="bg-deep-dark border border-gray-700 text-white text-sm rounded pl-8 pr-7 py-1 focus:outline-none w-56"
                            />
                            {search && (
                                <button
                                    onClick={() => setSearch('')}
                                    className="absolute right-1.5 top-1/2 -translate-y-1/2 text-gray-500 hover:text-white"
                                    title="Clear search"
                                >
                                    <X className="w-3.5 h-3.5" />
                                </button>
                            )}
                        </div>
                        <select
                            value={streamFilter}
                            onChange={e => setStreamFilter(e.target.value as StreamFilter)}
                            className="bg-deep-dark border border-gray-700 text-white text-sm rounded px-2 py-1 focus:outline-none"
                        >
                            <option value="all">All streams</option>
                            <option value="stdout">stdout</option>
                            <option value="stderr">stderr</option>
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
                            <p className="text-gray-500 italic">
                                {debouncedSearch || streamFilter !== 'all' ? 'No logs match the filter' : 'No logs'}
                            </p>
                        ) : (
                            entries.map((e, i) => (
                                <div key={i} className={`whitespace-pre-wrap break-all ${lineColor(e)}`}>
                                    <span className="text-gray-600 select-none">{formatTs(e.ts_ms)} </span>
                                    {highlightLine(e.line || ' ', debouncedSearch)}
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
