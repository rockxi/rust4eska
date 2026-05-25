import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Server, Trash2, X, Plus, Edit3, Container, Settings } from 'lucide-react';
import apiClient from '../api/client';

interface AppConfig {
    name: string;
    node_selector: string;
}

interface ContainerConfig {
    image: string;
    restart: string;
    command?: string[];
    ports?: string[];
}

interface SystemdConfig {
    exec: string;
    user?: string;
    working_dir?: string;
}

interface IngressConfig {
    domain: string;
    container_port: number;
}

interface Manifest {
    app: AppConfig;
    container?: ContainerConfig;
    systemd?: SystemdConfig;
    ingress?: IngressConfig;
    env: Record<string, string>;
}

const emptyManifest = (): Manifest => ({
    app: { name: '', node_selector: '' },
    container: { image: '', restart: 'always', ports: [] },
    systemd: undefined,
    ingress: undefined,
    env: {},
});

const fetchManifests = async (): Promise<Record<string, Manifest>> => {
    const response = await apiClient.get('/manifests');
    return response.data;
};

const upsertManifest = async (manifest: Manifest): Promise<void> => {
    await apiClient.post('/manifests', manifest);
};

const deleteManifest = async (name: string): Promise<void> => {
    await apiClient.delete(`/manifests?name=${encodeURIComponent(name)}`);
};

const Manifests: React.FC = () => {
    const queryClient = useQueryClient();
    const [editing, setEditing] = useState<Manifest | null>(null);
    const [isNew, setIsNew] = useState(false);
    const [envInput, setEnvInput] = useState('');

    const { data: manifestsMap, isLoading, isError } = useQuery({
        queryKey: ['manifests'],
        queryFn: fetchManifests,
        refetchInterval: 5000,
    });

    const manifests = manifestsMap
        ? Object.values(manifestsMap).sort((a, b) => a.app.name.localeCompare(b.app.name))
        : [];

    const upsertMutation = useMutation({
        mutationFn: upsertManifest,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['manifests'] });
            setEditing(null);
            setEnvInput('');
        },
    });

    const deleteMutation = useMutation({
        mutationFn: deleteManifest,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['manifests'] });
        },
    });

    const handleNew = () => {
        setEditing(emptyManifest());
        setIsNew(true);
        setEnvInput('');
    };

    const handleEdit = (m: Manifest) => {
        setEditing(JSON.parse(JSON.stringify(m)));
        setIsNew(false);
        setEnvInput(
            Object.entries(m.env)
                .map(([k, v]) => `${k}=${v}`)
                .join('\n')
        );
    };

    const handleSave = () => {
        if (!editing) return;
        if (!editing.app.node_selector.trim()) {
            alert('node_selector is required. Enter a node name (e.g. "agent1") or "all".');
            return;
        }
        const manifest = { ...editing };
        // Parse env from textarea
        const env: Record<string, string> = {};
        envInput.split('\n').forEach(line => {
            const idx = line.indexOf('=');
            if (idx > 0) {
                env[line.slice(0, idx).trim()] = line.slice(idx + 1).trim();
            }
        });
        manifest.env = env;

        // Parse ports from comma-separated string (stored as array in state)
        upsertMutation.mutate(manifest);
    };

    const handleDelete = (name: string) => {
        if (window.confirm(`Delete manifest "${name}"?`)) {
            deleteMutation.mutate(name);
        }
    };

    const updateField = (path: string[], value: unknown) => {
        if (!editing) return;
        const next = JSON.parse(JSON.stringify(editing)) as Manifest;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        let cur: any = next;
        for (let i = 0; i < path.length - 1; i++) {
            cur = cur[path[i]];
        }
        cur[path[path.length - 1]] = value;
        setEditing(next);
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="flex justify-between items-center mb-8">
                <div>
                    <h1 className="text-3xl font-bold text-white tracking-tight">Manifests</h1>
                    <p className="text-text-silver mt-2">Cluster state — workloads deployed to nodes</p>
                </div>
                <button
                    onClick={handleNew}
                    className="bg-accent-teal hover:bg-accent-teal/80 text-deep-dark px-4 py-2 rounded-lg font-bold flex items-center gap-2 transition-all"
                >
                    <Plus className="w-5 h-5" />
                    New Manifest
                </button>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load manifests.</p>
                </div>
            ) : manifests.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <Server className="w-12 h-12 text-gray-600 mx-auto mb-4" />
                    <p className="text-gray-400 text-lg">No manifests yet. Create one to deploy a workload.</p>
                </div>
            ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
                    {manifests.map((m) => {
                        const kind = m.container ? 'container' : m.systemd ? 'systemd' : 'unknown';
                        const detail = m.container?.image ?? m.systemd?.exec ?? '—';
                        return (
                            <div
                                key={m.app.name}
                                className="bg-slate-dark border border-gray-800 rounded-lg p-5 flex flex-col hover:border-accent-teal/30 transition-colors"
                            >
                                <div className="flex items-start gap-3 mb-4">
                                    <div className="w-10 h-10 rounded bg-deep-dark border border-gray-700 flex items-center justify-center shrink-0">
                                        {kind === 'container' ? (
                                            <Container className="w-5 h-5 text-accent-teal" />
                                        ) : (
                                            <Settings className="w-5 h-5 text-accent-teal" />
                                        )}
                                    </div>
                                    <div className="min-w-0">
                                        <h3 className="text-lg font-bold text-white truncate">{m.app.name}</h3>
                                        <p className="text-xs text-gray-500 truncate font-mono">{detail}</p>
                                        <span className="text-xs text-gray-600">
                                            <span className="text-accent-teal/70">{kind}</span>
                                            {' · '}node: {m.app.node_selector}
                                        </span>
                                    </div>
                                </div>

                                {Object.keys(m.env).length > 0 && (
                                    <div className="mb-3 text-xs text-gray-500 font-mono">
                                        {Object.keys(m.env).slice(0, 3).map(k => (
                                            <div key={k} className="truncate">{k}=***</div>
                                        ))}
                                        {Object.keys(m.env).length > 3 && (
                                            <div className="text-gray-600">+{Object.keys(m.env).length - 3} more</div>
                                        )}
                                    </div>
                                )}

                                <div className="mt-auto flex gap-2">
                                    <button
                                        onClick={() => handleEdit(m)}
                                        className="flex-1 bg-gray-800 hover:bg-gray-700 text-white py-2 rounded flex items-center justify-center gap-2 text-sm font-medium transition-colors"
                                    >
                                        <Edit3 className="w-4 h-4" />
                                        Edit
                                    </button>
                                    <button
                                        onClick={() => handleDelete(m.app.name)}
                                        disabled={deleteMutation.isPending}
                                        className="px-3 bg-gray-800 hover:bg-red-900/50 text-red-400 py-2 rounded flex items-center justify-center transition-colors"
                                        title="Delete"
                                    >
                                        <Trash2 className="w-4 h-4" />
                                    </button>
                                </div>
                            </div>
                        );
                    })}
                </div>
            )}

            {editing && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-xl shadow-2xl w-full max-w-2xl max-h-[90vh] flex flex-col overflow-hidden">
                        <div className="flex justify-between items-center p-5 border-b border-gray-800/50">
                            <h2 className="text-xl font-bold text-white">
                                {isNew ? 'New Manifest' : `Edit: ${editing.app.name}`}
                            </h2>
                            <button onClick={() => setEditing(null)} className="text-gray-400 hover:text-white">
                                <X className="w-6 h-6" />
                            </button>
                        </div>

                        <div className="flex-1 overflow-y-auto p-6 space-y-5">
                            {/* App section */}
                            <section>
                                <h3 className="text-sm font-semibold text-accent-teal mb-3 uppercase tracking-wider">[app]</h3>
                                <div className="grid grid-cols-2 gap-3">
                                    <div>
                                        <label className="block text-xs text-gray-400 mb-1">name</label>
                                        <input
                                            type="text"
                                            value={editing.app.name}
                                            onChange={e => updateField(['app', 'name'], e.target.value)}
                                            disabled={!isNew}
                                            className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50 disabled:opacity-50"
                                            placeholder="my-app"
                                        />
                                    </div>
                                    <div>
                                        <label className="block text-xs text-gray-400 mb-1">
                                            node_selector <span className="text-red-400">*</span>
                                        </label>
                                        <input
                                            type="text"
                                            value={editing.app.node_selector}
                                            onChange={e => updateField(['app', 'node_selector'], e.target.value)}
                                            className={`w-full bg-deep-dark border rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50 ${
                                                !editing.app.node_selector.trim() ? 'border-red-500' : 'border-gray-700'
                                            }`}
                                            placeholder="agent1 or all"
                                        />
                                        {!editing.app.node_selector.trim() && (
                                            <p className="text-red-400 text-xs mt-1">Required: enter a node name or "all"</p>
                                        )}
                                    </div>
                                </div>
                            </section>

                            {/* Container section */}
                            <section>
                                <div className="flex items-center gap-3 mb-3">
                                    <h3 className="text-sm font-semibold text-accent-teal uppercase tracking-wider">[container]</h3>
                                    <input
                                        type="checkbox"
                                        checked={!!editing.container}
                                        onChange={e => updateField(['container'], e.target.checked ? { image: '', restart: 'always' } : undefined)}
                                        className="accent-teal-400"
                                    />
                                </div>
                                {editing.container && (
                                    <div className="space-y-3">
                                        <div>
                                            <label className="block text-xs text-gray-400 mb-1">image</label>
                                            <input
                                                type="text"
                                                value={editing.container.image}
                                                onChange={e => updateField(['container', 'image'], e.target.value)}
                                                className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50"
                                                placeholder="nginx:latest"
                                            />
                                        </div>
                                        <div className="grid grid-cols-2 gap-3">
                                            <div>
                                                <label className="block text-xs text-gray-400 mb-1">restart</label>
                                                <select
                                                    value={editing.container.restart}
                                                    onChange={e => updateField(['container', 'restart'], e.target.value)}
                                                    className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm focus:outline-none focus:border-accent-teal/50"
                                                >
                                                    <option value="always">always</option>
                                                    <option value="on-failure">on-failure</option>
                                                    <option value="never">never</option>
                                                </select>
                                            </div>
                                            <div>
                                                <label className="block text-xs text-gray-400 mb-1">ports (host:container)</label>
                                                <input
                                                    type="text"
                                                    value={(editing.container.ports ?? []).join(', ')}
                                                    onChange={e => updateField(['container', 'ports'], e.target.value.split(',').map(s => s.trim()).filter(Boolean))}
                                                    className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50"
                                                    placeholder="8080:80, 443:443"
                                                />
                                            </div>
                                        </div>
                                    </div>
                                )}
                            </section>

                            {/* Systemd section */}
                            <section>
                                <div className="flex items-center gap-3 mb-3">
                                    <h3 className="text-sm font-semibold text-accent-teal uppercase tracking-wider">[systemd]</h3>
                                    <input
                                        type="checkbox"
                                        checked={!!editing.systemd}
                                        onChange={e => updateField(['systemd'], e.target.checked ? { exec: '' } : undefined)}
                                        className="accent-teal-400"
                                    />
                                </div>
                                {editing.systemd && (
                                    <div>
                                        <label className="block text-xs text-gray-400 mb-1">exec</label>
                                        <input
                                            type="text"
                                            value={editing.systemd.exec}
                                            onChange={e => updateField(['systemd', 'exec'], e.target.value)}
                                            className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50"
                                            placeholder="/usr/local/bin/my-service"
                                        />
                                    </div>
                                )}
                            </section>

                            {/* Ingress section */}
                            <section>
                                <div className="flex items-center gap-3 mb-3">
                                    <h3 className="text-sm font-semibold text-accent-teal uppercase tracking-wider">[ingress]</h3>
                                    <input
                                        type="checkbox"
                                        checked={!!editing.ingress}
                                        onChange={e => updateField(['ingress'], e.target.checked ? { domain: '', container_port: 80 } : undefined)}
                                        className="accent-teal-400"
                                    />
                                </div>
                                {editing.ingress && (
                                    <div className="grid grid-cols-2 gap-3">
                                        <div>
                                            <label className="block text-xs text-gray-400 mb-1">domain</label>
                                            <input
                                                type="text"
                                                value={editing.ingress.domain}
                                                onChange={e => updateField(['ingress', 'domain'], e.target.value)}
                                                className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50"
                                                placeholder="my-app.master.local"
                                            />
                                        </div>
                                        <div>
                                            <label className="block text-xs text-gray-400 mb-1">container_port</label>
                                            <input
                                                type="number"
                                                value={editing.ingress.container_port}
                                                onChange={e => updateField(['ingress', 'container_port'], parseInt(e.target.value) || 80)}
                                                className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50"
                                            />
                                        </div>
                                    </div>
                                )}
                            </section>

                            {/* Env section */}
                            <section>
                                <h3 className="text-sm font-semibold text-accent-teal mb-3 uppercase tracking-wider">[env]</h3>
                                <label className="block text-xs text-gray-400 mb-1">key=value per line · use vault://config/key for secrets</label>
                                <textarea
                                    value={envInput}
                                    onChange={e => setEnvInput(e.target.value)}
                                    rows={5}
                                    className="w-full bg-deep-dark border border-gray-700 rounded px-3 py-2 text-white text-sm font-mono focus:outline-none focus:border-accent-teal/50 resize-none"
                                    placeholder={"DATABASE_URL=postgres://...\nAPI_KEY=vault://default/api-key"}
                                />
                            </section>
                        </div>

                        <div className="p-5 border-t border-gray-800/50 flex justify-end gap-3">
                            <button
                                onClick={() => setEditing(null)}
                                className="px-5 py-2 border border-gray-700 hover:bg-gray-800 text-white rounded-lg transition-colors"
                            >
                                Cancel
                            </button>
                            <button
                                onClick={handleSave}
                                disabled={upsertMutation.isPending || !editing.app.name || !editing.app.node_selector.trim()}
                                className="bg-accent-teal hover:bg-accent-teal/90 disabled:opacity-50 text-deep-dark px-6 py-2 rounded-lg font-bold flex items-center gap-2 transition-all"
                            >
                                {upsertMutation.isPending ? (
                                    <div className="animate-spin rounded-full h-5 w-5 border-t-2 border-b-2 border-deep-dark" />
                                ) : null}
                                Save
                            </button>
                        </div>

                        {upsertMutation.isError && (
                            <div className="px-5 pb-4 text-red-400 text-sm">
                                Error: {String(upsertMutation.error)}
                            </div>
                        )}
                    </div>
                </div>
            )}
        </div>
    );
};

export default Manifests;
