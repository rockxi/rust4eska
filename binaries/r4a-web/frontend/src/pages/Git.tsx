import React, { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { GitBranch, Plus, Copy, X, Check } from 'lucide-react';
import apiClient from '../api/client';

interface RepoInfo {
    name: string;
    clone_url: string;
}

const fetchRepos = async (): Promise<RepoInfo[]> => {
    const response = await apiClient.get('/git/repos');
    return response.data;
};

const createRepo = async (name: string): Promise<{ name: string }> => {
    const response = await apiClient.post('/git/repos', { name });
    return response.data;
};

const Git: React.FC = () => {
    const queryClient = useQueryClient();
    const [isModalOpen, setIsModalOpen] = useState(false);
    const [newRepoName, setNewRepoName] = useState('');
    const [copiedUrl, setCopiedUrl] = useState<string | null>(null);

    const { data: repos, isLoading, isError } = useQuery({
        queryKey: ['git-repos'],
        queryFn: fetchRepos,
    });

    const mutation = useMutation({
        mutationFn: createRepo,
        onSuccess: () => {
            queryClient.invalidateQueries({ queryKey: ['git-repos'] });
            setIsModalOpen(false);
            setNewRepoName('');
        },
    });

    const handleCreateRepo = (e: React.FormEvent) => {
        e.preventDefault();
        if (newRepoName.trim()) {
            mutation.mutate(newRepoName.trim());
        }
    };

    const copyToClipboard = (url: string) => {
        navigator.clipboard.writeText(url);
        setCopiedUrl(url);
        setTimeout(() => setCopiedUrl(null), 2000);
    };

    return (
        <div className="p-6 max-w-7xl mx-auto">
            <div className="flex justify-between items-center mb-8">
                <div>
                    <h1 className="text-3xl font-bold text-white tracking-tight">Git Repositories</h1>
                    <p className="text-text-silver mt-2">Manage your bare Git repositories</p>
                </div>
                <button
                    onClick={() => setIsModalOpen(true)}
                    className="flex items-center gap-2 bg-accent-teal text-deep-dark px-4 py-2 rounded font-medium hover:bg-accent-teal/90 transition-colors"
                >
                    <Plus className="w-5 h-5" />
                    New Repository
                </button>
            </div>

            {isLoading ? (
                <div className="flex items-center justify-center h-64">
                    <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal"></div>
                </div>
            ) : isError ? (
                <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
                    <p className="text-red-400">Failed to load repositories. Please check your connection.</p>
                </div>
            ) : !repos || repos.length === 0 ? (
                <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
                    <GitBranch className="w-12 h-12 text-gray-600 mx-auto mb-4" />
                    <p className="text-gray-400 text-lg">No repositories found.</p>
                    <p className="text-gray-500 text-sm mt-2">Create one to get started.</p>
                </div>
            ) : (
                <div className="grid grid-cols-1 gap-4">
                    {repos.map((repo) => (
                        <div key={repo.name} className="bg-slate-dark border border-gray-800 rounded-lg p-5 shadow-lg flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                            <div className="flex items-center gap-3">
                                <div className="w-10 h-10 rounded bg-deep-dark border border-gray-700 flex items-center justify-center">
                                    <GitBranch className="w-5 h-5 text-accent-teal" />
                                </div>
                                <div>
                                    <h3 className="text-lg font-bold text-white">{repo.name}</h3>
                                </div>
                            </div>
                            <div className="flex items-center gap-2 bg-deep-dark border border-gray-700 rounded px-3 py-2 w-full sm:w-auto">
                                <code className="text-sm text-gray-300 font-mono truncate max-w-[200px] sm:max-w-xs">
                                    git clone {repo.clone_url}
                                </code>
                                <button
                                    onClick={() => copyToClipboard(`git clone ${repo.clone_url}`)}
                                    className="text-gray-400 hover:text-white transition-colors ml-2 p-1"
                                    title="Copy clone command"
                                >
                                    {copiedUrl === `git clone ${repo.clone_url}` ? (
                                        <Check className="w-4 h-4 text-green-400" />
                                    ) : (
                                        <Copy className="w-4 h-4" />
                                    )}
                                </button>
                            </div>
                        </div>
                    ))}
                </div>
            )}

            {isModalOpen && (
                <div className="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50 p-4">
                    <div className="bg-slate-dark border border-gray-800 rounded-lg shadow-2xl w-full max-w-md overflow-hidden">
                        <div className="flex justify-between items-center p-5 border-b border-gray-800">
                            <h2 className="text-xl font-bold text-white">Create Repository</h2>
                            <button 
                                onClick={() => setIsModalOpen(false)}
                                className="text-gray-400 hover:text-white transition-colors"
                            >
                                <X className="w-5 h-5" />
                            </button>
                        </div>
                        <form onSubmit={handleCreateRepo} className="p-5">
                            <div className="mb-5">
                                <label htmlFor="repoName" className="block text-sm font-medium text-text-silver mb-2">
                                    Repository Name
                                </label>
                                <input
                                    id="repoName"
                                    type="text"
                                    value={newRepoName}
                                    onChange={(e) => setNewRepoName(e.target.value)}
                                    placeholder="e.g., my-awesome-project"
                                    className="w-full bg-deep-dark border border-gray-700 rounded px-4 py-2 text-white focus:outline-none focus:border-accent-teal focus:ring-1 focus:ring-accent-teal transition-colors"
                                    autoFocus
                                    required
                                />
                            </div>
                            <div className="flex justify-end gap-3">
                                <button
                                    type="button"
                                    onClick={() => setIsModalOpen(false)}
                                    className="px-4 py-2 rounded text-gray-300 hover:bg-gray-800 transition-colors"
                                >
                                    Cancel
                                </button>
                                <button
                                    type="submit"
                                    disabled={mutation.isPending || !newRepoName.trim()}
                                    className="bg-accent-teal text-deep-dark px-4 py-2 rounded font-medium hover:bg-accent-teal/90 transition-colors disabled:opacity-50 disabled:cursor-not-allowed flex items-center gap-2"
                                >
                                    {mutation.isPending ? (
                                        <>
                                            <div className="animate-spin rounded-full h-4 w-4 border-t-2 border-b-2 border-deep-dark"></div>
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

export default Git;
