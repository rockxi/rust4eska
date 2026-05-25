import React from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { RefreshCw, Download, Server, CheckCircle, AlertCircle, Clock } from 'lucide-react';
import apiClient from '../api/client';

interface AgentStatus {
    status: string;
    checksum: string | null;
}

interface UpdateStatus {
    master_checksum: string | null;
    update_pending: boolean;
    agents: Record<string, AgentStatus>;
}

const fetchUpdateStatus = async (): Promise<UpdateStatus> => {
    const response = await apiClient.get('/update/status');
    return response.data;
};

const triggerUpdate = async () => {
    await apiClient.post('/update/trigger');
};

const fetchFromGithub = async () => {
    await apiClient.post('/update/fetch-github');
};

const Updates: React.FC = () => {
    const queryClient = useQueryClient();

    const { data: status, isLoading, isError } = useQuery({
        queryKey: ['updateStatus'],
        queryFn: fetchUpdateStatus,
        refetchInterval: 5000,
    });

    const triggerMutation = useMutation({
        mutationFn: triggerUpdate,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['updateStatus'] });
        },
    });

    const fetchMutation = useMutation({
        mutationFn: fetchFromGithub,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['updateStatus'] });
        },
    });

    const getStatusIcon = (agentStatus: string) => {
        switch (agentStatus.toLowerCase()) {
            case 'updated':
                return <CheckCircle className="w-5 h-5 text-accent-teal" />;
            case 'updating':
                return <RefreshCw className="w-5 h-5 text-accent-blue animate-spin" />;
            case 'pending':
                return <Clock className="w-5 h-5 text-yellow-500" />;
            case 'failed':
                return <AlertCircle className="w-5 h-5 text-red-500" />;
            case 'idle':
                return <Server className="w-5 h-5 text-gray-400" />;
            default:
                return <AlertCircle className="w-5 h-5 text-gray-500" />;
        }
    };

    const getStatusColor = (agentStatus: string) => {
        switch (agentStatus.toLowerCase()) {
            case 'updated':
                return 'text-accent-teal';
            case 'updating':
                return 'text-accent-blue';
            case 'pending':
                return 'text-yellow-500';
            case 'failed':
                return 'text-red-500';
            case 'idle':
                return 'text-gray-400';
            default:
                return 'text-gray-500';
        }
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="flex justify-between items-center mb-8">
                <div>
                    <h1 className="text-3xl font-bold text-white tracking-tight flex items-center gap-3">
                        <RefreshCw className="w-8 h-8 text-accent-teal" />
                        Cluster Updates
                    </h1>
                    <p className="text-text-silver mt-2">Manage and monitor cluster-wide updates</p>
                </div>
                <div className="flex gap-4">
                    <button
                        onClick={() => fetchMutation.mutate()}
                        disabled={fetchMutation.isPending}
                        className="bg-slate-dark hover:bg-gray-800 border border-gray-700 text-white font-bold py-2 px-4 rounded flex items-center gap-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                        {fetchMutation.isPending ? (
                            <RefreshCw className="w-5 h-5 animate-spin" />
                        ) : (
                            <Download className="w-5 h-5" />
                        )}
                        Fetch from GitHub
                    </button>
                    <button
                        onClick={() => triggerMutation.mutate()}
                        disabled={triggerMutation.isPending || status?.update_pending}
                        className="bg-accent-teal hover:bg-accent-teal/80 text-deep-dark font-bold py-2 px-4 rounded flex items-center gap-2 transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
                    >
                        {triggerMutation.isPending ? (
                            <RefreshCw className="w-5 h-5 animate-spin" />
                        ) : (
                            <RefreshCw className="w-5 h-5" />
                        )}
                        Trigger Cluster Update
                    </button>
                </div>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load update status.</p>
                </div>
            ) : (
                <div className="space-y-6">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg p-6 shadow-lg flex flex-col md:flex-row md:items-center justify-between gap-4">
                        <div>
                            <h2 className="text-lg font-bold text-white mb-1">Master Status</h2>
                            <div className="flex items-center gap-2">
                                <span className="text-gray-400 text-sm">Checksum:</span>
                                <span className="font-mono text-accent-teal bg-deep-dark px-2 py-1 rounded text-sm">
                                    {status?.master_checksum ? status.master_checksum.substring(0, 12) : 'Unknown'}
                                </span>
                            </div>
                        </div>
                        <div className="flex items-center gap-3">
                            <span className="text-gray-400 text-sm">Update Pending:</span>
                            {status?.update_pending ? (
                                <span className="flex items-center gap-1.5 text-yellow-500 bg-yellow-500/10 px-3 py-1.5 rounded-full text-sm font-medium border border-yellow-500/20">
                                    <Clock className="w-4 h-4" /> Yes
                                </span>
                            ) : (
                                <span className="flex items-center gap-1.5 text-accent-teal bg-accent-teal/10 px-3 py-1.5 rounded-full text-sm font-medium border border-accent-teal/20">
                                    <CheckCircle className="w-4 h-4" /> No
                                </span>
                            )}
                        </div>
                    </div>

                    <div className="bg-slate-dark border border-gray-800 rounded-lg overflow-hidden shadow-lg">
                        <div className="p-4 border-b border-gray-800 bg-gray-800/30">
                            <h2 className="text-lg font-bold text-white flex items-center gap-2">
                                <Server className="w-5 h-5 text-accent-blue" />
                                Agent Status
                            </h2>
                        </div>
                        <table className="w-full text-left border-collapse">
                            <thead>
                                <tr className="bg-gray-800/50 border-b border-gray-800 text-gray-400 text-sm uppercase tracking-wider">
                                    <th className="p-4 font-medium">Agent IP</th>
                                    <th className="p-4 font-medium">Status</th>
                                    <th className="p-4 font-medium">Checksum</th>
                                </tr>
                            </thead>
                            <tbody className="divide-y divide-gray-800">
                                {!status?.agents || Object.keys(status.agents).length === 0 ? (
                                    <tr>
                                        <td colSpan={3} className="p-8 text-center text-gray-500">
                                            No agents found.
                                        </td>
                                    </tr>
                                ) : (
                                    Object.entries(status.agents).map(([ip, agentStatus]) => (
                                        <tr key={ip} className="hover:bg-gray-800/30 transition-colors">
                                            <td className="p-4 text-white font-medium font-mono text-sm">
                                                {ip}
                                            </td>
                                            <td className="p-4">
                                                <div className="flex items-center gap-2">
                                                    {getStatusIcon(agentStatus.status)}
                                                    <span className={`font-medium capitalize ${getStatusColor(agentStatus.status)}`}>
                                                        {agentStatus.status}
                                                    </span>
                                                </div>
                                            </td>
                                            <td className="p-4 text-gray-400 font-mono text-sm">
                                                {agentStatus.checksum ? agentStatus.checksum.substring(0, 12) : 'Unknown'}
                                            </td>
                                        </tr>
                                    ))
                                )}
                            </tbody>
                        </table>
                    </div>
                </div>
            )}
        </div>
    );
};

export default Updates;
