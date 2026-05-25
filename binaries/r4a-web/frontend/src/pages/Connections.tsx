import React from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Wifi, WifiOff, Clock, Trash2 } from 'lucide-react';
import apiClient from '../api/client';

interface Connection {
    id: string;
    pubkey: string;
    vpn_ip: string;
    label: string | null;
    connected_at: number;
    last_seen: number;
}

const fetchConnections = async (): Promise<Connection[]> => {
    const r = await apiClient.get('/connections');
    return r.data;
};

const deleteConnection = async (id: string): Promise<void> => {
    await apiClient.delete(`/connections/${id}`);
};

function formatAge(secs: number): string {
    const now = Math.floor(Date.now() / 1000);
    const diff = now - secs;
    if (diff < 60) return `${diff}s ago`;
    if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
    return `${Math.floor(diff / 3600)}h ago`;
}

function isStale(lastSeen: number): boolean {
    return Math.floor(Date.now() / 1000) - lastSeen > 90;
}

const Connections: React.FC = () => {
    const queryClient = useQueryClient();

    const { data: connections, isLoading, isError } = useQuery({
        queryKey: ['connections'],
        queryFn: fetchConnections,
        refetchInterval: 5000,
    });

    const disconnectMutation = useMutation({
        mutationFn: deleteConnection,
        onSuccess: () => queryClient.invalidateQueries({ queryKey: ['connections'] }),
    });

    if (isLoading) return <div className="text-gray-400 p-4">Loading connections...</div>;
    if (isError) return <div className="text-red-400 p-4">Failed to load connections.</div>;

    return (
        <div className="p-4">
            <h2 className="text-xl font-bold text-white mb-4 flex items-center gap-2">
                <Wifi size={20} /> Client Connections
            </h2>

            {connections && connections.length === 0 && (
                <div className="text-gray-500 text-sm">No active connections.</div>
            )}

            <div className="space-y-2">
                {connections?.map(conn => {
                    const stale = isStale(conn.last_seen);
                    return (
                        <div
                            key={conn.id}
                            className={`flex items-center justify-between rounded-lg px-4 py-3 bg-gray-800 border ${stale ? 'border-yellow-700' : 'border-gray-700'}`}
                        >
                            <div className="flex items-center gap-3">
                                {stale
                                    ? <WifiOff size={16} className="text-yellow-400" />
                                    : <Wifi size={16} className="text-green-400" />
                                }
                                <div>
                                    <div className="text-white font-mono text-sm">
                                        {conn.vpn_ip}
                                        {conn.label && (
                                            <span className="ml-2 text-gray-400 font-sans">({conn.label})</span>
                                        )}
                                    </div>
                                    <div className="text-xs text-gray-500 flex gap-3 mt-0.5">
                                        <span className="flex items-center gap-1">
                                            <Clock size={10} /> connected {formatAge(conn.connected_at)}
                                        </span>
                                        <span className={stale ? 'text-yellow-500' : 'text-gray-500'}>
                                            last seen {formatAge(conn.last_seen)}
                                        </span>
                                        <span className="text-gray-600 font-mono">{conn.id.slice(0, 8)}…</span>
                                    </div>
                                </div>
                            </div>

                            <button
                                onClick={() => disconnectMutation.mutate(conn.id)}
                                disabled={disconnectMutation.isPending}
                                className="p-1.5 rounded hover:bg-red-900/40 text-red-400 hover:text-red-300 transition-colors"
                                title="Disconnect"
                            >
                                <Trash2 size={14} />
                            </button>
                        </div>
                    );
                })}
            </div>
        </div>
    );
};

export default Connections;
