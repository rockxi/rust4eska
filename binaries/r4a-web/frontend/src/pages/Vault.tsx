import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Lock, Plus, Eye, EyeOff, Edit2, Trash2, X, Key } from 'lucide-react';
import apiClient from '../api/client';

interface VaultConfig {
    id: string;
    name: string;
    created_at: number;
}

const fetchVaultConfigs = async (): Promise<VaultConfig[]> => {
    const response = await apiClient.get('/vault/configs');
    return response.data;
};

const createVaultConfig = async (name: string): Promise<VaultConfig> => {
    const response = await apiClient.post('/vault/configs', { name });
    return response.data;
};

const fetchVaultKeys = async (configId: string): Promise<string[]> => {
    const response = await apiClient.get(`/vault/list?config_id=${encodeURIComponent(configId)}`);
    return response.data;
};

const fetchVaultValue = async (configId: string, key: string): Promise<string> => {
    const response = await apiClient.get(`/vault?config_id=${encodeURIComponent(configId)}&key=${encodeURIComponent(key)}`);
    return response.data;
};

const saveVaultSecret = async ({ configId, key, value }: { configId: string; key: string; value: string }): Promise<void> => {
    await apiClient.post('/vault', { config_id: configId, key, value });
};

const deleteVaultSecret = async ({ configId, key }: { configId: string; key: string }): Promise<void> => {
    await apiClient.delete(`/vault?config_id=${encodeURIComponent(configId)}&key=${encodeURIComponent(key)}`);
};

const SecretItem = ({ configId, secretKey, onDelete, onEdit }: { configId: string, secretKey: string, onDelete: (key: string) => void, onEdit: (key: string, value: string) => void }) => {
    const [isRevealed, setIsRevealed] = useState(false);
    const [value, setValue] = useState<string | null>(null);
    const [isLoading, setIsLoading] = useState(false);

    const handleToggleReveal = async () => {
        if (isRevealed) {
            setIsRevealed(false);
        } else {
            if (value === null) {
                setIsLoading(true);
                try {
                    const fetchedValue = await fetchVaultValue(configId, secretKey);
                    setValue(fetchedValue);
                } catch (error) {
                    console.error("Failed to fetch secret value", error);
                } finally {
                    setIsLoading(false);
                }
            }
            setIsRevealed(true);
        }
    };

    const handleEdit = async () => {
        let currentValue = value;
        if (currentValue === null) {
            try {
                currentValue = await fetchVaultValue(configId, secretKey);
                setValue(currentValue);
            } catch (error) {
                console.error("Failed to fetch secret value for editing", error);
                return;
            }
        }
        onEdit(secretKey, currentValue || '');
    };

    return (
        <div className="bg-slate-dark border border-gray-800 rounded-lg p-4 shadow-lg flex flex-col sm:flex-row sm:items-center justify-between gap-4">
            <div className="flex items-center gap-3">
                <div className="w-10 h-10 rounded bg-deep-dark border border-gray-700 flex items-center justify-center shrink-0">
                    <Key className="w-5 h-5 text-accent-teal" />
                </div>
                <div className="overflow-hidden">
                    <h3 className="text-lg font-bold text-white truncate">{secretKey}</h3>
                    <div className="text-sm text-gray-400 font-mono mt-1 flex items-center gap-2">
                        {isLoading ? (
                            <span className="animate-pulse">Loading...</span>
                        ) : isRevealed ? (
                            <span className="break-all">{value}</span>
                        ) : (
                            <span>••••••••••••••••</span>
                        )}
                    </div>
                </div>
            </div>
            <div className="flex items-center gap-2 shrink-0">
                <button
                    onClick={handleToggleReveal}
                    className="p-2 text-gray-400 hover:text-white hover:bg-gray-800 rounded transition-colors"
                    title={isRevealed ? "Hide value" : "Reveal value"}
                >
                    {isRevealed ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                </button>
                <button
                    onClick={handleEdit}
                    className="p-2 text-gray-400 hover:text-accent-blue hover:bg-gray-800 rounded transition-colors"
                    title="Edit secret"
                >
                    <Edit2 className="w-4 h-4" />
                </button>
                <button
                    onClick={() => onDelete(secretKey)}
                    className="p-2 text-gray-400 hover:text-red-400 hover:bg-gray-800 rounded transition-colors"
                    title="Delete secret"
                >
                    <Trash2 className="w-4 h-4" />
                </button>
            </div>
        </div>
    );
};

const Vault: React.FC = () => {
    const queryClient = useQueryClient();
    const [isModalOpen, setIsModalOpen] = useState(false);
    const [isEditing, setIsEditing] = useState(false);
    const [secretKey, setSecretKey] = useState('');
    const [secretValue, setSecretValue] = useState('');
    
    const [isConfigModalOpen, setIsConfigModalOpen] = useState(false);
    const [newConfigName, setNewConfigName] = useState('');
    const [selectedConfigId, setSelectedConfigId] = useState<string>('default');

    const { data: configs, isLoading: isConfigsLoading } = useQuery({
        queryKey: ['vault-configs'],
        queryFn: fetchVaultConfigs,
    });

    const { data: keys, isLoading: isKeysLoading, isError } = useQuery({
        queryKey: ['vault-keys', selectedConfigId],
        queryFn: () => fetchVaultKeys(selectedConfigId),
        enabled: !!selectedConfigId,
    });

    const saveMutation = useMutation({
        mutationFn: saveVaultSecret,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['vault-keys', selectedConfigId] });
            closeModal();
        },
    });

    const deleteMutation = useMutation({
        mutationFn: deleteVaultSecret,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['vault-keys', selectedConfigId] });
        },
    });

    const configCreateMutation = useMutation({
        mutationFn: createVaultConfig,
        onSuccess: (newConfig) => {
            queryClient.invalidateQueries({ queryKey: ['vault-configs'] });
            setSelectedConfigId(newConfig.id);
            closeConfigModal();
        },
    });

    const openAddModal = () => {
        setIsEditing(false);
        setSecretKey('');
        setSecretValue('');
        setIsModalOpen(true);
    };

    const openEditModal = (key: string, value: string) => {
        setIsEditing(true);
        setSecretKey(key);
        setSecretValue(value);
        setIsModalOpen(true);
    };

    const closeModal = () => {
        setIsModalOpen(false);
        setSecretKey('');
        setSecretValue('');
    };

    const openConfigModal = () => {
        setNewConfigName('');
        setIsConfigModalOpen(true);
    };

    const closeConfigModal = () => {
        setIsConfigModalOpen(false);
    };

    const handleSave = (e: React.FormEvent) => {
        e.preventDefault();
        if (secretKey.trim() && secretValue.trim()) {
            saveMutation.mutate({ configId: selectedConfigId, key: secretKey.trim(), value: secretValue.trim() });
        }
    };

    const handleDelete = (key: string) => {
        if (window.confirm(`Are you sure you want to delete the secret "${key}"?`)) {
            deleteMutation.mutate({ configId: selectedConfigId, key });
        }
    };

    const handleConfigCreate = (e: React.FormEvent) => {
        e.preventDefault();
        if (newConfigName.trim()) {
            configCreateMutation.mutate(newConfigName.trim());
        }
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="flex flex-col md:flex-row justify-between items-start md:items-center gap-4 mb-8">
                <div>
                    <h1 className="text-3xl font-bold text-white tracking-tight">Vault</h1>
                    <p className="text-text-silver mt-2">Manage your cluster secrets securely</p>
                </div>
                <div className="flex gap-2">
                    <button
                        onClick={openConfigModal}
                        className="flex items-center gap-2 bg-deep-dark border border-gray-700 text-white px-4 py-2 rounded font-medium hover:bg-gray-800 transition-colors"
                    >
                        <Plus className="w-5 h-5 text-accent-blue" />
                        New Config
                    </button>
                    <button
                        onClick={openAddModal}
                        className="flex items-center gap-2 bg-accent-teal text-deep-dark px-4 py-2 rounded font-medium hover:bg-accent-teal/90 transition-colors"
                    >
                        <Plus className="w-5 h-5" />
                        Add Secret
                    </button>
                </div>
            </div>

            <div className="mb-6 flex overflow-x-auto pb-2 gap-2 scrollbar-hide">
                {configs?.map((config) => (
                    <button
                        key={config.id}
                        onClick={() => setSelectedConfigId(config.id)}
                        className={`px-4 py-2 rounded-full text-sm font-medium transition-all whitespace-nowrap ${
                            selectedConfigId === config.id
                                ? 'bg-accent-teal text-deep-dark shadow-lg shadow-accent-teal/20'
                                : 'bg-slate-dark text-gray-400 border border-gray-800 hover:border-gray-600'
                        }`}
                    >
                        {config.name}
                    </button>
                ))}
                {isConfigsLoading && <div className="h-10 w-32 bg-slate-dark animate-pulse rounded-full"></div>}
            </div>

            {isKeysLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load secrets. Please check your connection.</p>
                </div>
            ) : !keys || keys.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <Lock className="w-12 h-12 text-gray-600 mx-auto mb-4" />
                    <p className="text-gray-400 text-lg">No secrets found in this config.</p>
                    <p className="text-gray-500 text-sm mt-2">Add a secret to get started.</p>
                </div>
            ) : (
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                    {keys.map((key) => (
                        <SecretItem 
                            key={`${selectedConfigId}-${key}`}
                            configId={selectedConfigId}
                            secretKey={key} 
                            onDelete={handleDelete} 
                            onEdit={openEditModal} 
                        />
                    ))}
                </div>
            )}

            {isModalOpen && (
                <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg shadow-2xl w-full max-w-md overflow-hidden">
                        <div className="flex justify-between items-center p-5 border-b border-gray-800">
                            <h2 className="text-xl font-bold text-white">
                                {isEditing ? 'Edit Secret' : 'Add Secret'}
                            </h2>
                            <button 
                                onClick={closeModal}
                                className="text-gray-400 hover:text-white transition-colors"
                            >
                                <X className="w-5 h-5" />
                            </button>
                        </div>
                        <form onSubmit={handleSave} className="p-5">
                            <div className="mb-4 text-sm text-gray-400 bg-deep-dark/50 p-2 rounded border border-gray-800">
                                Saving to: <span className="text-accent-teal font-bold">{configs?.find(c => c.id === selectedConfigId)?.name || selectedConfigId}</span>
                            </div>
                            <div className="mb-4">
                                <label htmlFor="secretKey" className="block text-sm font-medium text-text-silver mb-2">
                                    Key
                                </label>
                                <input
                                    id="secretKey"
                                    type="text"
                                    value={secretKey}
                                    onChange={(e) => setSecretKey(e.target.value)}
                                    placeholder="e.g., DATABASE_URL"
                                    className="w-full bg-deep-dark border border-gray-700 rounded px-4 py-2 text-white focus:outline-none focus:border-accent-teal focus:ring-1 focus:ring-accent-teal transition-colors disabled:opacity-50"
                                    autoFocus={!isEditing}
                                    disabled={isEditing}
                                    required
                                />
                            </div>
                            <div className="mb-6">
                                <label htmlFor="secretValue" className="block text-sm font-medium text-text-silver mb-2">
                                    Value
                                </label>
                                <textarea
                                    id="secretValue"
                                    value={secretValue}
                                    onChange={(e) => setSecretValue(e.target.value)}
                                    placeholder="Enter secret value..."
                                    className="w-full bg-deep-dark border border-gray-700 rounded px-4 py-2 text-white focus:outline-none focus:border-accent-teal focus:ring-1 focus:ring-accent-teal transition-colors min-h-[100px] resize-y"
                                    autoFocus={isEditing}
                                    required
                                />
                            </div>
                            <div className="flex justify-end gap-3">
                                <button
                                    type="button"
                                    onClick={closeModal}
                                    className="px-4 py-2 rounded text-gray-300 hover:bg-gray-800 transition-colors"
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    disabled={saveMutation.isPending || !secretKey.trim() || !secretValue.trim()}
                                    className="bg-accent-teal text-deep-dark px-4 py-2 rounded font-medium hover:bg-accent-teal/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                                >
                                    {saveMutation.isPending ? (
                                        <>
                                            <div className="animate-spin rounded-full h-4 w-4 border-t-2 border-b-2 border-deep-dark"></div>
                                            Saving...
                                        </>
                                    ) : (
                                        'Save'
                                    )}
                                </button>
                            </div>
                        </form>
                    </div>
                </div>
            )}

            {isConfigModalOpen && (
                <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg shadow-2xl w-full max-w-md overflow-hidden">
                        <div className="flex justify-between items-center p-5 border-b border-gray-800">
                            <h2 className="text-xl font-bold text-white">Create New Vault Config</h2>
                            <button 
                                onClick={closeConfigModal}
                                className="text-gray-400 hover:text-white transition-colors"
                            >
                                <X className="w-5 h-5" />
                            </button>
                        </div>
                        <form onSubmit={handleConfigCreate} className="p-5">
                            <div className="mb-6">
                                <label htmlFor="configName" className="block text-sm font-medium text-text-silver mb-2">
                                    Config Name
                                </label>
                                <input
                                    id="configName"
                                    type="text"
                                    value={newConfigName}
                                    onChange={(e) => setNewConfigName(e.target.value)}
                                    placeholder="e.g., Production, Staging"
                                    className="w-full bg-deep-dark border border-gray-700 rounded px-4 py-2 text-white focus:outline-none focus:border-accent-teal focus:ring-1 focus:ring-accent-teal transition-colors"
                                    autoFocus
                                    required
                                />
                            </div>
                            <div className="flex justify-end gap-3">
                                <button
                                    type="button"
                                    onClick={closeConfigModal}
                                    className="px-4 py-2 rounded text-gray-300 hover:bg-gray-800 transition-colors"
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    disabled={configCreateMutation.isPending || !newConfigName.trim()}
                                    className="bg-accent-blue text-white px-4 py-2 rounded font-medium hover:bg-accent-blue/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                                >
                                    {configCreateMutation.isPending ? (
                                        <>
                                            <div className="animate-spin rounded-full h-4 w-4 border-t-2 border-b-2 border-white"></div>
                                            Creating...
                                        </>
                                    ) : (
                                        'Create'
                                    )}
                                </button>
                            </div>
                        </form>
                    </div>
                </div>
            )}
        </div>
    );
};

export default Vault;
