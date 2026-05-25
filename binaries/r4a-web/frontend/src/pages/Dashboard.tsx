import React from 'react';
import { useQuery } from '@tanstack/react-query';
import { Box, Server, Globe, X, Activity } from 'lucide-react';
import apiClient from '../api/client';

interface NodeInfo {
    ip: string;
    name: string;
    role: string;
    cpu_percent: number | null;
    ram_used_mb: number | null;
    ram_total_mb: number | null;
    vram_used_mb: number | null;
    vram_total_mb: number | null;
    last_seen: number | null;
}

const fetchNodes = async (): Promise<NodeInfo[]> => {
    const response = await apiClient.get('/nodes');
    return response.data;
};

interface Manifest {
    app: { name: string; node_selector: string; };
    container?: { image: string; restart: string; command?: string[]; ports?: string[]; };
    systemd?: { exec: string; user?: string; working_dir?: string; };
    ingress?: { domain: string; container_port: number; };
    env: Record<string, string>;
}

const fetchNodeManifests = async (nodeName: string): Promise<Manifest[]> => {
    const response = await apiClient.get(`/manifests?node=${nodeName}`);
    const data = response.data;
    if (Array.isArray(data)) {
        return data;
    }
    if (typeof data === 'object' && data !== null) {
        return Object.values(data);
    }
    return [];
};

const NodeManifestsModal = ({ nodeName, onClose }: { nodeName: string, onClose: () => void }) => {
    const { data: manifests, isLoading, isError } = useQuery({
        queryKey: ['manifests', nodeName],
        queryFn: () => fetchNodeManifests(nodeName),
        enabled: !!nodeName,
    });

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-4">
            <div className="bg-slate-dark border border-gray-800 rounded-xl shadow-2xl w-full max-w-3xl max-h-[85vh] flex flex-col overflow-hidden animate-in fade-in zoom-in-95 duration-200">
                <div className="flex justify-between items-center p-5 border-b border-gray-800/50 bg-slate-dark/80">
                    <h2 className="text-xl font-bold text-white flex items-center gap-2">
                        <Server className="w-5 h-5 text-accent-teal" />
                        Services on {nodeName}
                    </h2>
                    <button 
                        onClick={onClose}
                        className="text-gray-400 hover:text-white transition-colors p-1 rounded-md hover:bg-gray-800"
                    >
                        <X className="w-5 h-5" />
                    </button>
                </div>
                
                <div className="p-5 overflow-y-auto flex-1">
                    {isLoading ? (
                        <div className="flex items-center justify-center h-40">
                            <div className="animate-spin rounded-full h-8 w-8 border-t-2 border-b-2 border-accent-teal"></div>
                        </div>
                    ) : isError ? (
                        <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                            <p className="text-red-400">Failed to load services for this node.</p>
                        </div>
                    ) : !manifests || manifests.length === 0 ? (
                        <div className="flex flex-col items-center justify-center h-40 text-gray-400">
                            <Activity className="w-10 h-10 mb-3 opacity-20" />
                            <p>No services running on this node.</p>
                        </div>
                    ) : (
                        <div className="grid gap-4">
                            {manifests.map((manifest, idx) => (
                                <div key={idx} className="bg-deep-dark border border-gray-800/60 rounded-lg p-4 hover:border-gray-700 transition-colors">
                                    <div className="flex justify-between items-start mb-3">
                                        <div className="flex items-center gap-2">
                                            <Box className="w-5 h-5 text-accent-blue" />
                                            <h3 className="text-lg font-semibold text-white">{manifest.app.name}</h3>
                                        </div>
                                        <span className="text-[10px] px-2 py-0.5 rounded-full font-medium bg-gray-800 text-gray-300 border border-gray-700 uppercase tracking-wider">
                                            {manifest.container ? 'Container' : manifest.systemd ? 'Systemd' : 'Unknown'}
                                        </span>
                                    </div>
                                    
                                    <div className="space-y-2 text-sm">
                                        {manifest.container && (
                                            <div className="flex items-start gap-2 text-gray-400">
                                                <span className="text-gray-500 w-16 shrink-0">Image:</span>
                                                <span className="font-mono text-gray-300 break-all">{manifest.container.image}</span>
                                            </div>
                                        )}
                                        {manifest.systemd && (
                                            <div className="flex items-start gap-2 text-gray-400">
                                                <span className="text-gray-500 w-16 shrink-0">Exec:</span>
                                                <span className="font-mono text-gray-300 break-all">{manifest.systemd.exec}</span>
                                            </div>
                                        )}
                                        {manifest.ingress && (
                                            <div className="flex items-start gap-2 text-gray-400">
                                                <span className="text-gray-500 w-16 shrink-0">Ingress:</span>
                                                <div className="flex items-center gap-1.5 text-accent-teal">
                                                    <Globe className="w-3.5 h-3.5" />
                                                    <a href={`https://${manifest.ingress.domain}`} target="_blank" rel="noreferrer" className="hover:underline">
                                                        {manifest.ingress.domain}
                                                    </a>
                                                </div>
                                            </div>
                                        )}
                                    </div>
                                </div>
                            ))}
                        </div>
                    )}
                </div>
            </div>
        </div>
    );
};

const ProgressBar = ({ label, value, max, unit = '%' }: { label: string, value: number | null, max: number | null, unit?: string }) => {
    if (value === null || max === null || max === 0) {
        return (
            <div className="mb-3">
                <div className="flex justify-between text-xs mb-1">
                    <span className="text-text-silver">{label}</span>
                    <span className="text-gray-500">N/A</span>
                </div>
                <div className="w-full bg-deep-dark rounded-full h-2">
                    <div className="bg-gray-700 h-2 rounded-full" style={{ width: '0%' }}></div>
                </div>
            </div>
        );
    }

    const percentage = Math.min(100, Math.max(0, (value / max) * 100));
    const displayValue = unit === '%' ? `${value.toFixed(1)}%` : `${value} / ${max} ${unit}`;

    return (
        <div className="mb-3">
            <div className="flex justify-between text-xs mb-1">
                <span className="text-text-silver">{label}</span>
                <span className="text-accent-teal">{displayValue}</span>
            </div>
            <div className="w-full bg-deep-dark rounded-full h-2">
                <div 
                    className="bg-accent-teal h-2 rounded-full transition-all duration-500 ease-in-out" 
                    style={{ width: `${percentage}%` }}
                ></div>
            </div>
        </div>
    );
};

const NodeCard = ({ node, currentTime, onClick }: { node: NodeInfo, currentTime: number, onClick: () => void }) => {
    const isOnline = node.last_seen !== null && (currentTime - node.last_seen) < 20;

    return (
        <div 
            className="bg-slate-dark border border-gray-800 rounded-lg p-5 shadow-lg flex flex-col cursor-pointer hover:border-gray-600 transition-colors group"
            onClick={onClick}
        >
            <div className="flex justify-between items-start mb-4">
                <div>
                    <h3 className="text-lg font-bold text-white flex items-center gap-2 group-hover:text-accent-teal transition-colors">
                        {node.name}
                        <span className={`text-[10px] px-2 py-0.5 rounded-full font-medium ${node.role.toLowerCase() === 'master' ? 'bg-accent-blue/20 text-accent-blue border border-accent-blue/30' : 'bg-gray-700 text-gray-300 border border-gray-600'}`}>
                            {node.role}
                        </span>
                    </h3>
                    <p className="text-sm text-gray-400 font-mono mt-1">{node.ip}</p>
                </div>
                <div className="flex items-center gap-1.5">
                    <div className={`w-2.5 h-2.5 rounded-full ${isOnline ? 'bg-accent-teal shadow-[0_0_8px_rgba(102,252,241,0.6)]' : 'bg-red-500'}`}></div>
                    <span className="text-xs text-gray-400">{isOnline ? 'Online' : 'Offline'}</span>
                </div>
            </div>

            <div className="mt-auto pt-4 border-t border-gray-800/50">
                <ProgressBar 
                    label="CPU Usage" 
                    value={node.cpu_percent} 
                    max={100} 
                />
                <ProgressBar 
                    label="RAM Usage" 
                    value={node.ram_used_mb} 
                    max={node.ram_total_mb} 
                    unit="MB" 
                />
                {(node.vram_total_mb !== null && node.vram_total_mb > 0) && (
                    <ProgressBar 
                        label="VRAM Usage" 
                        value={node.vram_used_mb} 
                        max={node.vram_total_mb} 
                        unit="MB" 
                    />
                )}
            </div>
        </div>
    );
};

const Dashboard: React.FC = () => {
    const [currentTime, setCurrentTime] = React.useState(() => Date.now() / 1000);
    const [selectedNode, setSelectedNode] = React.useState<string | null>(null);

    React.useEffect(() => {
        const interval = setInterval(() => {
            setCurrentTime(Date.now() / 1000);
        }, 1000);
        return () => clearInterval(interval);
    }, []);

    const { data: nodes, isLoading, isError } = useQuery({
        queryKey: ['nodes'],
        queryFn: fetchNodes,
        refetchInterval: 2000,
    });

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="mb-8">
                <h1 className="text-3xl font-bold text-white tracking-tight">Cluster Dashboard</h1>
                <p className="text-text-silver mt-2">Real-time node status and resource usage</p>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load node data. Please check your connection.</p>
                </div>
            ) : !nodes || nodes.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <p className="text-gray-400 text-lg">No nodes found in the cluster.</p>
                </div>
            ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                    {nodes.map((node) => (
                        <NodeCard 
                            key={node.ip} 
                            node={node} 
                            currentTime={currentTime} 
                            onClick={() => setSelectedNode(node.name)}
                        />
                    ))}
                </div>
            )}

            {selectedNode && (
                <NodeManifestsModal 
                    nodeName={selectedNode} 
                    onClose={() => setSelectedNode(null)} 
                />
            )}
        </div>
    );
};

export default Dashboard;