import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Container, RefreshCw, ScrollText, X, ChevronDown, ChevronRight, Square, Play } from 'lucide-react';
import apiClient from '../api/client';

interface NodeInfo {
    name: string;
    ip: string;
    role: string;
}

interface ContainerInfo {
    id: string;
    name: string;
    image: string;
    status: string;
    state: string;
}

const fetchNodes = async (): Promise<NodeInfo[]> => {
    const r = await apiClient.get('/nodes');
    return r.data;
};

const fetchContainers = async (node: string): Promise<ContainerInfo[]> => {
    const r = await apiClient.get(`/nodes/${encodeURIComponent(node)}/containers`);
    return r.data;
};

const fetchLogs = async (node: string, container: string, tail: number): Promise<string[]> => {
    const r = await apiClient.get(`/nodes/${encodeURIComponent(node)}/containers/${encodeURIComponent(container)}/logs?tail=${tail}`);
    return r.data;
};

const restartContainer = async ({ node, container }: { node: string; container: string }) => {
    await apiClient.post(`/nodes/${encodeURIComponent(node)}/containers/${encodeURIComponent(container)}/restart`);
};

const stopContainer = async ({ node, container }: { node: string; container: string }) => {
    await apiClient.post(`/nodes/${encodeURIComponent(node)}/containers/${encodeURIComponent(container)}/stop`);
};

const startContainer = async ({ node, container }: { node: string; container: string }) => {
    await apiClient.post(`/nodes/${encodeURIComponent(node)}/containers/${encodeURIComponent(container)}/start`);
};

function stateColor(state: string) {
    if (state === 'running') return 'text-green-400';
    if (state === 'exited') return 'text-red-400';
    if (state === 'restarting') return 'text-yellow-400';
    return 'text-gray-400';
}

const NodeContainers: React.FC<{ node: NodeInfo; onLogs: (node: string, container: string) => void }> = ({ node, onLogs }) => {
    const [open, setOpen] = useState(true);
    const queryClient = useQueryClient();

    const { data: containers, isLoading, isError } = useQuery({
        queryKey: ['containers', node.name],
        queryFn: () => fetchContainers(node.name),
        refetchInterval: 8000,
        enabled: open,
    });

    const restartMutation = useMutation({
        mutationFn: restartContainer,
        onSuccess: (_, { node: n }) => {
            queryClient.invalidateQueries({ queryKey: ['containers', n] });
        },
    });

    const stopMutation = useMutation({
        mutationFn: stopContainer,
        onSuccess: (_, { node: n }) => {
            queryClient.invalidateQueries({ queryKey: ['containers', n] });
        },
    });

    const startMutation = useMutation({
        mutationFn: startContainer,
        onSuccess: (_, { node: n }) => {
            queryClient.invalidateQueries({ queryKey: ['containers', n] });
        },
    });

    const anyPending = restartMutation.isPending || stopMutation.isPending || startMutation.isPending;

    return (
        <div className="bg-slate-dark border border-gray-800 rounded-lg overflow-hidden">
            <button
                onClick={() => setOpen(o => !o)}
                className="w-full flex items-center gap-3 px-5 py-4 hover:bg-deep-dark/50 transition-colors"
            >
                {open ? <ChevronDown className="w-4 h-4 text-gray-400" /> : <ChevronRight className="w-4 h-4 text-gray-400" />}
                <Container className="w-4 h-4 text-accent-teal" />
                <span className="font-bold text-white">{node.name}</span>
                <span className="text-xs text-gray-500 font-mono">{node.ip}</span>
                <span className="ml-auto text-xs px-2 py-0.5 rounded bg-gray-800 text-gray-400">
                    {node.role}
                </span>
            </button>

            {open && (
                <div className="border-t border-gray-800">
                    {isLoading ? (
                        <div className="flex items-center justify-center py-8">
                            <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-accent-teal" />
                        </div>
                    ) : isError ? (
                        <p className="text-red-400 text-sm px-5 py-4">Failed to load containers (agent API may not be reachable)</p>
                    ) : !containers || containers.length === 0 ? (
                        <p className="text-gray-500 text-sm px-5 py-4">No r4a-managed containers on this node</p>
                    ) : (
                        <table className="w-full text-sm">
                            <thead>
                                <tr className="text-xs text-gray-500 uppercase border-b border-gray-800">
                                    <th className="text-left px-5 py-2">Name</th>
                                    <th className="text-left px-5 py-2">Image</th>
                                    <th className="text-left px-5 py-2">State</th>
                                    <th className="text-left px-5 py-2">Status</th>
                                    <th className="px-5 py-2" />
                                </tr>
                            </thead>
                            <tbody>
                                {containers.map(c => (
                                    <tr key={c.id} className="border-b border-gray-800/50 hover:bg-deep-dark/30">
                                        <td className="px-5 py-3 font-mono text-white">{c.name}</td>
                                        <td className="px-5 py-3 text-gray-400 font-mono text-xs truncate max-w-[200px]">{c.image}</td>
                                        <td className={`px-5 py-3 font-mono font-semibold ${stateColor(c.state)}`}>{c.state}</td>
                                        <td className="px-5 py-3 text-gray-500 text-xs">{c.status}</td>
                                        <td className="px-5 py-3">
                                            <div className="flex gap-2 justify-end">
                                                <button
                                                    onClick={() => onLogs(node.name, c.name)}
                                                    className="flex items-center gap-1 px-3 py-1.5 bg-gray-800 hover:bg-gray-700 text-gray-300 rounded text-xs transition-colors"
                                                >
                                                    <ScrollText className="w-3.5 h-3.5" />
                                                    Logs
                                                </button>
                                                {c.state === 'running' ? (
                                                    <button
                                                        onClick={() => stopMutation.mutate({ node: node.name, container: c.name })}
                                                        disabled={anyPending}
                                                        className="flex items-center gap-1 px-3 py-1.5 bg-gray-800 hover:bg-red-900/50 text-red-400 rounded text-xs transition-colors disabled:opacity-50"
                                                    >
                                                        <Square className="w-3.5 h-3.5" />
                                                        Stop
                                                    </button>
                                                ) : (
                                                    <button
                                                        onClick={() => startMutation.mutate({ node: node.name, container: c.name })}
                                                        disabled={anyPending}
                                                        className="flex items-center gap-1 px-3 py-1.5 bg-gray-800 hover:bg-green-900/50 text-green-400 rounded text-xs transition-colors disabled:opacity-50"
                                                    >
                                                        <Play className="w-3.5 h-3.5" />
                                                        Start
                                                    </button>
                                                )}
                                                <button
                                                    onClick={() => restartMutation.mutate({ node: node.name, container: c.name })}
                                                    disabled={anyPending}
                                                    className="flex items-center gap-1 px-3 py-1.5 bg-gray-800 hover:bg-yellow-900/50 text-yellow-400 rounded text-xs transition-colors disabled:opacity-50"
                                                >
                                                    <RefreshCw className={`w-3.5 h-3.5 ${restartMutation.isPending ? 'animate-spin' : ''}`} />
                                                    Restart
                                                </button>
                                            </div>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    )}
                </div>
            )}
        </div>
    );
};

const LogsModal: React.FC<{ node: string; container: string; onClose: () => void }> = ({ node, container, onClose }) => {
    const [tail, setTail] = useState(200);
    const logRef = React.useRef<HTMLDivElement>(null);

    const { data: lines, isLoading, refetch } = useQuery({
        queryKey: ['logs', node, container, tail],
        queryFn: () => fetchLogs(node, container, tail),
        refetchInterval: 5000,
    });

    React.useEffect(() => {
        if (logRef.current) {
            logRef.current.scrollTop = logRef.current.scrollHeight;
        }
    }, [lines]);

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4">
            <div className="bg-slate-dark border border-gray-800 rounded-xl shadow-2xl w-full max-w-5xl h-[80vh] flex flex-col">
                <div className="flex items-center gap-3 px-5 py-4 border-b border-gray-800/50 shrink-0">
                    <ScrollText className="w-4 h-4 text-accent-teal" />
                    <span className="font-bold text-white font-mono">{container}</span>
                    <span className="text-gray-500 text-sm">on {node}</span>
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
                            onClick={() => refetch()}
                            className="p-1.5 text-gray-400 hover:text-white hover:bg-gray-800 rounded transition-colors"
                            title="Refresh"
                        >
                            <RefreshCw className="w-4 h-4" />
                        </button>
                        <button onClick={onClose} className="p-1.5 text-gray-400 hover:text-white hover:bg-gray-800 rounded transition-colors">
                            <X className="w-5 h-5" />
                        </button>
                    </div>
                </div>

                <div
                    ref={logRef}
                    className="flex-1 overflow-y-auto p-4 font-mono text-xs leading-5 bg-deep-dark"
                >
                    {isLoading ? (
                        <div className="flex items-center justify-center h-full">
                            <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-accent-teal" />
                        </div>
                    ) : !lines || lines.length === 0 ? (
                        <p className="text-gray-500 italic">No logs</p>
                    ) : (
                        lines.map((line, i) => (
                            <div key={i} className={`whitespace-pre-wrap break-all ${line.toLowerCase().includes('error') || line.toLowerCase().includes('err]') ? 'text-red-400' : line.toLowerCase().includes('warn') ? 'text-yellow-400' : 'text-gray-300'}`}>
                                {line || ' '}
                            </div>
                        ))
                    )}
                </div>
            </div>
        </div>
    );
};

const Containers: React.FC = () => {
    const [logsTarget, setLogsTarget] = useState<{ node: string; container: string } | null>(null);

    const { data: nodes, isLoading } = useQuery({
        queryKey: ['nodes'],
        queryFn: fetchNodes,
        refetchInterval: 10000,
    });

    const workloadNodes = nodes ?? [];

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="mb-8">
                <h1 className="text-3xl font-bold text-white tracking-tight">Containers</h1>
                <p className="text-text-silver mt-2">Running workloads on cluster nodes</p>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal" />
                </div>
            ) : workloadNodes.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <Container className="w-12 h-12 text-gray-600 mx-auto mb-4" />
                    <p className="text-gray-400 text-lg">No nodes connected</p>
                </div>
            ) : (
                <div className="space-y-4">
                    {workloadNodes.map(node => (
                        <NodeContainers
                            key={node.name}
                            node={node}
                            onLogs={(n, c) => setLogsTarget({ node: n, container: c })}
                        />
                    ))}
                </div>
            )}

            {logsTarget && (
                <LogsModal
                    node={logsTarget.node}
                    container={logsTarget.container}
                    onClose={() => setLogsTarget(null)}
                />
            )}
        </div>
    );
};

export default Containers;
