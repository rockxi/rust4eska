import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Plus, Trash2, Key, Shield, X } from 'lucide-react';
import apiClient from '../api/client';

interface Token {
    id: string;
    username: string;
    created_at: number;
}

interface BasicUser {
    username: string;
}

const VERBS = ['get', 'list', 'create', 'update', 'delete', 'all'];
const RESOURCES = ['nodes', 'manifests', 'vault', 'git_repos', 'registry', 'tokens', 'policies', 'bindings', 'all'];

const fetchTokens = async (): Promise<Token[]> => {
    const response = await apiClient.get('/tokens');
    return response.data;
};

const fetchUsers = async (): Promise<BasicUser[]> => {
    const response = await apiClient.get('/users');
    return response.data;
};

const deleteUser = async (username: string) => {
    await apiClient.delete(`/users?username=${encodeURIComponent(username)}`);
};

const deleteToken = async (id: string) => {
    await apiClient.delete(`/tokens?id=${id}`);
};

const createToken = async (data: { username: string; verbs: string[]; resources: string[] }) => {
    const response = await apiClient.post('/tokens', data);
    return response.data;
};

const createUser = async (data: { username: string; password: string; verbs: string[]; resources: string[] }) => {
    const response = await apiClient.post('/users', data);
    return response.data;
};

const RBAC: React.FC = () => {
    const queryClient = useQueryClient();
    const [isModalOpen, setIsModalOpen] = useState(false);
    const [isUserModalOpen, setIsUserModalOpen] = useState(false);
    const [newTokenUsername, setNewTokenUsername] = useState('');
    const [selectedVerbs, setSelectedVerbs] = useState<string[]>([]);
    const [selectedResources, setSelectedResources] = useState<string[]>([]);
    const [createdTokenId, setCreatedTokenId] = useState<string | null>(null);
    const [newUserUsername, setNewUserUsername] = useState('');
    const [newUserPassword, setNewUserPassword] = useState('');

    const { data: tokens, isLoading, isError } = useQuery({
        queryKey: ['tokens'],
        queryFn: fetchTokens,
    });

    const { data: users } = useQuery({
        queryKey: ['basic-users'],
        queryFn: fetchUsers,
    });

    const deleteMutation = useMutation({
        mutationFn: deleteToken,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['tokens'] });
        },
    });

    const createMutation = useMutation({
        mutationFn: createToken,
        onSuccess: (data) => {
            queryClient.invalidateQueries({ queryKey: ['tokens'] });
            setCreatedTokenId(data.id || 'Token created successfully');
        },
    });

    const createUserMutation = useMutation({
        mutationFn: createUser,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['basic-users'] });
            setIsUserModalOpen(false);
            setNewUserUsername('');
            setNewUserPassword('');
        },
    });

    const deleteUserMutation = useMutation({
        mutationFn: deleteUser,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['basic-users'] });
        },
    });

    const handleCreateToken = (e: React.FormEvent) => {
        e.preventDefault();
        if (!newTokenUsername) return;
        
        createMutation.mutate({
            username: newTokenUsername,
            verbs: selectedVerbs.length > 0 ? selectedVerbs : ['all'],
            resources: selectedResources.length > 0 ? selectedResources : ['all'],
        });
    };

    const toggleSelection = (item: string, list: string[], setList: React.Dispatch<React.SetStateAction<string[]>>) => {
        if (item === 'all') {
            if (list.includes('all')) {
                setList([]);
            } else {
                setList(['all']);
            }
            return;
        }

        let newList = [...list];
        if (newList.includes('all')) {
            newList = newList.filter(i => i !== 'all');
        }

        if (newList.includes(item)) {
            newList = newList.filter(i => i !== item);
        } else {
            newList.push(item);
        }
        setList(newList);
    };

    const resetModal = () => {
        setIsModalOpen(false);
        setNewTokenUsername('');
        setSelectedVerbs([]);
        setSelectedResources([]);
        setCreatedTokenId(null);
        createMutation.reset();
    };

    const resetUserModal = () => {
        setIsUserModalOpen(false);
        setNewUserUsername('');
        setNewUserPassword('');
        createUserMutation.reset();
    };

    const formatDate = (timestamp: number) => {
        return new Date(timestamp * 1000).toLocaleString();
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="flex justify-between items-center mb-8">
                <div>
                    <h1 className="text-3xl font-bold text-white tracking-tight flex items-center gap-3">
                        <Shield className="w-8 h-8 text-accent-teal" />
                        RBAC & Tokens
                    </h1>
                    <p className="text-text-silver mt-2">Manage access tokens and permissions</p>
                </div>
                <button
                    onClick={() => setIsModalOpen(true)}
                    className="bg-accent-teal hover:bg-accent-teal/80 text-deep-dark font-bold py-2 px-4 rounded flex items-center gap-2 transition-colors"
                >
                    <Plus className="w-5 h-5" />
                    Create Token
                </button>
                <button
                    onClick={() => setIsUserModalOpen(true)}
                    className="bg-gray-800 hover:bg-gray-700 text-white font-bold py-2 px-4 rounded flex items-center gap-2 transition-colors"
                >
                    <Key className="w-5 h-5" />
                    Create Registry User
                </button>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load tokens.</p>
                </div>
            ) : (
                <div className="bg-slate-dark border border-gray-800 rounded-lg overflow-hidden shadow-lg">
                    <table className="w-full text-left border-collapse">
                        <thead>
                            <tr className="bg-gray-800/50 border-b border-gray-800 text-gray-400 text-sm uppercase tracking-wider">
                                <th className="p-4 font-medium">Username</th>
                                <th className="p-4 font-medium">Token ID (Prefix)</th>
                                <th className="p-4 font-medium">Created At</th>
                                <th className="p-4 font-medium text-right">Actions</th>
                            </tr>
                        </thead>
                        <tbody className="divide-y divide-gray-800">
                            {tokens?.length === 0 ? (
                                <tr>
                                    <td colSpan={4} className="p-8 text-center text-gray-500">
                                        No tokens found.
                                    </td>
                                </tr>
                            ) : (
                                tokens?.map((token) => (
                                    <tr key={token.id} className="hover:bg-gray-800/30 transition-colors">
                                        <td className="p-4 text-white font-medium flex items-center gap-2">
                                            <Key className="w-4 h-4 text-accent-blue" />
                                            {token.username}
                                        </td>
                                        <td className="p-4 text-gray-400 font-mono text-sm">
                                            {token.id.substring(0, 8)}...
                                        </td>
                                        <td className="p-4 text-gray-400 text-sm">
                                            {formatDate(token.created_at)}
                                        </td>
                                        <td className="p-4 text-right">
                                            <button
                                                onClick={() => {
                                                    if (window.confirm(`Are you sure you want to delete token for ${token.username}?`)) {
                                                        deleteMutation.mutate(token.id);
                                                    }
                                                }}
                                                className="text-gray-500 hover:text-red-400 transition-colors p-2 rounded hover:bg-red-400/10"
                                                title="Delete Token"
                                            >
                                                <Trash2 className="w-5 h-5" />
                                            </button>
                                        </td>
                                    </tr>
                                ))
                            )}
                        </tbody>
                    </table>
                </div>
            )}

            <div className="mt-8 bg-slate-dark border border-gray-800 rounded-lg overflow-hidden shadow-lg">
                <div className="p-4 border-b border-gray-800">
                    <h2 className="text-lg font-bold text-white">Basic Auth Users</h2>
                    <p className="text-sm text-gray-500 mt-1">Use these credentials for `docker login` against the registry.</p>
                </div>
                <table className="w-full text-left border-collapse">
                    <thead>
                        <tr className="bg-gray-800/50 border-b border-gray-800 text-gray-400 text-sm uppercase tracking-wider">
                            <th className="p-4 font-medium">Username</th>
                            <th className="p-4 font-medium text-right">Actions</th>
                        </tr>
                    </thead>
                    <tbody className="divide-y divide-gray-800">
                        {!users || users.length === 0 ? (
                            <tr>
                                <td colSpan={2} className="p-8 text-center text-gray-500">No basic-auth users found.</td>
                            </tr>
                        ) : (
                            users.map((user) => (
                                <tr key={user.username} className="hover:bg-gray-800/30 transition-colors">
                                    <td className="p-4 text-white font-medium">{user.username}</td>
                                    <td className="p-4 text-right">
                                        <button
                                            onClick={() => {
                                                if (window.confirm(`Delete basic-auth user ${user.username}?`)) {
                                                    deleteUserMutation.mutate(user.username);
                                                }
                                            }}
                                            className="text-gray-500 hover:text-red-400 transition-colors p-2 rounded hover:bg-red-400/10"
                                            title="Delete User"
                                        >
                                            <Trash2 className="w-5 h-5" />
                                        </button>
                                    </td>
                                </tr>
                            ))
                        )}
                    </tbody>
                </table>
            </div>

            {isModalOpen && (
                <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg shadow-2xl w-full max-w-2xl overflow-hidden flex flex-col max-h-[90vh]">
                        <div className="flex justify-between items-center p-6 border-b border-gray-800">
                            <h2 className="text-xl font-bold text-white">Create New Token</h2>
                            <button onClick={resetModal} className="text-gray-400 hover:text-white transition-colors">
                                <X className="w-6 h-6" />
                            </button>
                        </div>

                        <div className="p-6 overflow-y-auto flex-1">
                            {createdTokenId ? (
                                <div className="bg-accent-teal/10 border border-accent-teal/30 rounded-lg p-6 text-center">
                                    <h3 className="text-accent-teal font-bold text-lg mb-2">Token Created Successfully!</h3>
                                    <p className="text-gray-300 mb-4 text-sm">Please copy this token now. You won't be able to see it again.</p>
                                    <div className="bg-deep-dark p-4 rounded border border-gray-700 font-mono text-accent-teal break-all select-all">
                                        {createdTokenId}
                                    </div>
                                </div>
                            ) : (
                                <form id="create-token-form" onSubmit={handleCreateToken} className="space-y-6">
                                    <div>
                                        <label className="block text-sm font-medium text-gray-300 mb-2">Username / Identifier</label>
                                        <input
                                            type="text"
                                            required
                                            value={newTokenUsername}
                                            onChange={(e) => setNewTokenUsername(e.target.value)}
                                            className="w-full bg-deep-dark border border-gray-700 rounded p-3 text-white focus:outline-none focus:border-accent-teal transition-colors"
                                            placeholder="e.g., ci-runner, admin-user"
                                        />
                                    </div>

                                    <div>
                                        <label className="block text-sm font-medium text-gray-300 mb-2">Allowed Verbs</label>
                                        <div className="flex flex-wrap gap-2">
                                            {VERBS.map(verb => (
                                                <button
                                                    key={verb}
                                                    type="button"
                                                    onClick={() => toggleSelection(verb, selectedVerbs, setSelectedVerbs)}
                                                    className={`px-3 py-1.5 rounded text-sm font-medium transition-colors border ${
                                                        selectedVerbs.includes(verb)
                                                            ? 'bg-accent-blue/20 border-accent-blue text-accent-teal'
                                                            : 'bg-deep-dark border-gray-700 text-gray-400 hover:border-gray-500'
                                                    }`}
                                                >
                                                    {verb}
                                                </button>
                                            ))}
                                        </div>
                                        {selectedVerbs.length === 0 && <p className="text-xs text-gray-500 mt-2">If none selected, defaults to 'all'.</p>}
                                    </div>

                                    <div>
                                        <label className="block text-sm font-medium text-gray-300 mb-2">Allowed Resources</label>
                                        <div className="flex flex-wrap gap-2">
                                            {RESOURCES.map(resource => (
                                                <button
                                                    key={resource}
                                                    type="button"
                                                    onClick={() => toggleSelection(resource, selectedResources, setSelectedResources)}
                                                    className={`px-3 py-1.5 rounded text-sm font-medium transition-colors border ${
                                                        selectedResources.includes(resource)
                                                            ? 'bg-accent-blue/20 border-accent-blue text-accent-teal'
                                                            : 'bg-deep-dark border-gray-700 text-gray-400 hover:border-gray-500'
                                                    }`}
                                                >
                                                    {resource}
                                                </button>
                                            ))}
                                        </div>
                                        {selectedResources.length === 0 && <p className="text-xs text-gray-500 mt-2">If none selected, defaults to 'all'.</p>}
                                    </div>
                                    
                                    {createMutation.isError && (
                                        <div className="text-red-400 text-sm bg-red-900/20 p-3 rounded border border-red-500/30">
                                            Failed to create token. Please try again.
                                        </div>
                                    )}
                                </form>
                            )}
                        </div>

                        <div className="p-6 border-t border-gray-800 flex justify-end gap-3 bg-gray-900/50">
                            <button
                                type="button"
                                onClick={resetModal}
                                className="px-4 py-2 rounded text-gray-300 hover:text-white hover:bg-gray-800 transition-colors"
                            >
                                {createdTokenId ? 'Close' : 'Cancel'}
                            </button>
                            {!createdTokenId && (
                                <button
                                    type="submit"
                                    form="create-token-form"
                                    disabled={createMutation.isPending || !newTokenUsername}
                                    className="bg-accent-teal hover:bg-accent-teal/80 text-deep-dark font-bold py-2 px-6 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                                >
                                    {createMutation.isPending ? (
                                        <>
                                            <div className="w-4 h-4 border-2 border-deep-dark border-t-transparent rounded-full animate-spin"></div>
                                            Creating...
                                        </>
                                    ) : (
                                        'Create Token'
                                    )}
                                </button>
                            )}
                        </div>
                    </div>
                </div>
            )}

            {isUserModalOpen && (
                <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg shadow-2xl w-full max-w-lg overflow-hidden">
                        <div className="flex justify-between items-center p-6 border-b border-gray-800">
                            <h2 className="text-xl font-bold text-white">Create Registry User</h2>
                            <button onClick={resetUserModal} className="text-gray-400 hover:text-white transition-colors">
                                <X className="w-6 h-6" />
                            </button>
                        </div>
                        <form
                            onSubmit={(e) => {
                                e.preventDefault();
                                createUserMutation.mutate({
                                    username: newUserUsername,
                                    password: newUserPassword,
                                    verbs: ['create', 'update', 'delete', 'list', 'get'],
                                    resources: ['registry'],
                                });
                            }}
                            className="p-6 space-y-5"
                        >
                            <div>
                                <label className="block text-sm font-medium text-gray-300 mb-2">Username</label>
                                <input
                                    type="text"
                                    required
                                    value={newUserUsername}
                                    onChange={(e) => setNewUserUsername(e.target.value)}
                                    className="w-full bg-deep-dark border border-gray-700 rounded p-3 text-white focus:outline-none focus:border-accent-teal transition-colors"
                                    placeholder="e.g. registry-ci"
                                />
                            </div>
                            <div>
                                <label className="block text-sm font-medium text-gray-300 mb-2">Password</label>
                                <input
                                    type="password"
                                    required
                                    minLength={8}
                                    value={newUserPassword}
                                    onChange={(e) => setNewUserPassword(e.target.value)}
                                    className="w-full bg-deep-dark border border-gray-700 rounded p-3 text-white focus:outline-none focus:border-accent-teal transition-colors"
                                    placeholder="At least 8 characters"
                                />
                            </div>
                            <div className="text-sm text-gray-500">
                                This user gets registry-only permissions for `docker login`, `docker push` and `docker pull`.
                            </div>
                            {createUserMutation.isError && (
                                <div className="text-red-400 text-sm bg-red-900/20 p-3 rounded border border-red-500/30">
                                    Failed to create user. Username may already exist.
                                </div>
                            )}
                            <div className="flex justify-end gap-3 pt-2">
                                <button
                                    type="button"
                                    onClick={resetUserModal}
                                    className="px-4 py-2 rounded text-gray-300 hover:text-white hover:bg-gray-800 transition-colors"
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    disabled={createUserMutation.isPending || !newUserUsername || !newUserPassword}
                                    className="bg-accent-teal hover:bg-accent-teal/80 text-deep-dark font-bold py-2 px-6 rounded transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                                >
                                    {createUserMutation.isPending ? 'Creating...' : 'Create User'}
                                </button>
                            </div>
                        </form>
                    </div>
                </div>
            )}
        </div>
    );
};

export default RBAC;
