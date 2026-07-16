import React, { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, Copy, HardDrive, Package, Tag, Trash2 } from 'lucide-react';
import axios from 'axios';
import apiClient from '../api/client';

interface RegistryRepoInfo {
  name: string;
  tag_count: number;
  total_size: number;
}

interface RegistryTagInfo {
  tag: string;
  digest: string;
  size: number;
  pushed_at: number;
}

const fetchRepos = async (): Promise<RegistryRepoInfo[]> => {
  const response = await apiClient.get('/registry/repos');
  return response.data;
};

const fetchRepoTags = async (repo: string): Promise<RegistryTagInfo[]> => {
  const response = await apiClient.get(`/registry/repos/${encodeURIComponent(repo)}/tags`);
  return response.data;
};

const deleteTag = async ({ repo, tag }: { repo: string; tag: string }): Promise<void> => {
  await apiClient.delete(`/registry/repos/${encodeURIComponent(repo)}/tags/${encodeURIComponent(tag)}`);
};

const formatBytes = (bytes: number) => {
  if (bytes <= 0) {
    return '0 B';
  }
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = bytes;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value >= 10 || index === 0 ? value.toFixed(0) : value.toFixed(1)} ${units[index]}`;
};

const formatDigest = (digest: string) => {
  if (digest.length <= 24) {
    return digest;
  }
  return `${digest.slice(0, 19)}...${digest.slice(-8)}`;
};

const formatPushedAt = (ts: number) => {
  if (!ts) {
    return '—';
  }
  return new Date(ts * 1000).toLocaleString();
};

const RepoCard: React.FC<{ repo: RegistryRepoInfo }> = ({ repo }) => {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();

  const {
    data: tags,
    isLoading,
    isError,
    error,
  } = useQuery({
    queryKey: ['registry-tags', repo.name],
    queryFn: () => fetchRepoTags(repo.name),
    enabled: open,
  });

  const deleteMutation = useMutation({
    mutationFn: deleteTag,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ['registry-repos'] });
      await queryClient.invalidateQueries({ queryKey: ['registry-tags', repo.name] });
    },
  });

  const deleteError = deleteMutation.error;

  return (
    <div className="bg-slate-dark border border-gray-800 rounded-lg overflow-hidden">
      <button
        onClick={() => setOpen((value) => !value)}
        className="w-full px-5 py-4 flex items-center gap-3 hover:bg-deep-dark/40 transition-colors text-left"
      >
        {open ? <ChevronDown className="w-4 h-4 text-gray-400" /> : <ChevronRight className="w-4 h-4 text-gray-400" />}
        <Package className="w-4 h-4 text-accent-teal" />
        <div className="min-w-0">
          <div className="font-bold text-white truncate">{repo.name}</div>
          <div className="text-xs text-gray-500">{repo.tag_count} tag(s)</div>
        </div>
        <div className="ml-auto flex items-center gap-2 text-xs text-gray-400 font-mono">
          <HardDrive className="w-4 h-4" />
          {formatBytes(repo.total_size)}
        </div>
      </button>

      {open && (
        <div className="border-t border-gray-800">
          {isLoading ? (
            <div className="flex items-center justify-center py-8">
              <div className="animate-spin rounded-full h-6 w-6 border-t-2 border-b-2 border-accent-teal" />
            </div>
          ) : isError ? (
            <div className="px-5 py-4 text-sm text-red-400">
              {axios.isAxiosError(error) && error.response?.status === 403
                ? 'Access denied for this registry repository.'
                : 'Failed to load tags.'}
            </div>
          ) : !tags || tags.length === 0 ? (
            <div className="px-5 py-4 text-sm text-gray-500">No tags found.</div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-xs text-gray-500 uppercase border-b border-gray-800">
                    <th className="text-left px-5 py-2">Tag</th>
                    <th className="text-left px-5 py-2">Digest</th>
                    <th className="text-left px-5 py-2">Size</th>
                    <th className="text-left px-5 py-2">Pushed</th>
                    <th className="px-5 py-2" />
                  </tr>
                </thead>
                <tbody>
                  {tags.map((tag) => {
                    const pending = deleteMutation.isPending
                      && deleteMutation.variables?.repo === repo.name
                      && deleteMutation.variables?.tag === tag.tag;
                    return (
                      <tr key={tag.tag} className="border-b border-gray-800/50 hover:bg-deep-dark/30">
                        <td className="px-5 py-3 font-mono text-white">
                          <div className="flex items-center gap-2">
                            <Tag className="w-3.5 h-3.5 text-gray-500" />
                            {tag.tag}
                          </div>
                        </td>
                        <td className="px-5 py-3 font-mono text-xs text-gray-400" title={tag.digest}>
                          {formatDigest(tag.digest)}
                        </td>
                        <td className="px-5 py-3 text-gray-300">{formatBytes(tag.size)}</td>
                        <td className="px-5 py-3 text-gray-500">{formatPushedAt(tag.pushed_at)}</td>
                        <td className="px-5 py-3">
                          <div className="flex justify-end">
                            <button
                              onClick={() => {
                                if (window.confirm(`Delete tag "${repo.name}:${tag.tag}"?`)) {
                                  deleteMutation.mutate({ repo: repo.name, tag: tag.tag });
                                }
                              }}
                              disabled={deleteMutation.isPending}
                              className="flex items-center gap-1 px-3 py-1.5 bg-gray-800 hover:bg-red-900/50 text-red-400 rounded text-xs transition-colors disabled:opacity-50"
                            >
                              <Trash2 className="w-3.5 h-3.5" />
                              {pending ? 'Deleting...' : 'Delete'}
                            </button>
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {deleteError && (
                <div className="px-5 py-3 text-sm text-red-400">
                  {axios.isAxiosError(deleteError) && deleteError.response?.status === 403
                    ? 'Delete is forbidden by RBAC.'
                    : 'Failed to delete tag.'}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

const Registry: React.FC = () => {
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  const { data: repos, isLoading, isError, error } = useQuery({
    queryKey: ['registry-repos'],
    queryFn: fetchRepos,
    refetchInterval: 10000,
  });

  const registryHost = `${window.location.hostname}:3501`;
  const exampleRepo = 'demo/myapp';
  const loginCommand = `docker login ${registryHost}`;
  const tagCommand = `docker tag nginx:alpine ${registryHost}/${exampleRepo}:latest`;
  const pushCommand = `docker push ${registryHost}/${exampleRepo}:latest`;
  const pullCommand = `docker pull ${registryHost}/${exampleRepo}:latest`;
  const manifestImage = `${registryHost}/${exampleRepo}:latest`;

  const copyCommand = async (command: string) => {
    await navigator.clipboard.writeText(command);
    setCopiedCommand(command);
    window.setTimeout(() => setCopiedCommand(null), 1500);
  };

  return (
    <div className="p-6 max-w-7xl mx-auto">
      <div className="flex justify-between items-center mb-8">
        <div>
          <h1 className="text-3xl font-bold text-white tracking-tight">Registry</h1>
          <p className="text-text-silver mt-2">Container repositories, tags and delete actions from Web UI</p>
        </div>
      </div>

      <div className="bg-slate-dark border border-gray-800 rounded-lg p-5 mb-6">
        <h2 className="text-lg font-bold text-white mb-2">How To Push</h2>
        <p className="text-sm text-gray-400 mb-4">
          Create a basic-auth user in <span className="text-white font-medium">RBAC</span>, then use standard Docker commands against this registry endpoint.
        </p>
        <div className="space-y-3">
          {[loginCommand, tagCommand, pushCommand, pullCommand].map((command) => (
            <div key={command} className="flex items-center gap-2 bg-deep-dark border border-gray-700 rounded px-3 py-2">
              <code className="text-sm text-gray-300 font-mono flex-1 overflow-x-auto">{command}</code>
              <button
                onClick={() => copyCommand(command)}
                className="text-gray-400 hover:text-white transition-colors p-1"
                title="Copy command"
              >
                <Copy className={`w-4 h-4 ${copiedCommand === command ? 'text-accent-teal' : ''}`} />
              </button>
            </div>
          ))}
        </div>
        <div className="mt-4 text-sm text-gray-500">
          Manifest image example: <code className="text-gray-300 font-mono">{manifestImage}</code>
        </div>
      </div>

      {isLoading ? (
        <div className="flex items-center justify-center h-64">
          <div className="animate-spin rounded-full h-12 w-12 border-t-2 border-b-2 border-accent-teal" />
        </div>
      ) : isError ? (
        <div className="bg-red-900/20 border border-red-500/50 rounded-lg p-6 text-center">
          <p className="text-red-400">
            {axios.isAxiosError(error) && error.response?.status === 403
              ? 'Access denied. You need Registry permissions.'
              : 'Failed to load registry repositories.'}
          </p>
        </div>
      ) : !repos || repos.length === 0 ? (
        <div className="bg-slate-dark border border-gray-800 rounded-lg p-12 text-center">
          <Package className="w-12 h-12 text-gray-600 mx-auto mb-4" />
          <p className="text-gray-400 text-lg">No registry repositories found.</p>
          <p className="text-gray-500 text-sm mt-2">Push at least one image to populate the registry.</p>
        </div>
      ) : (
        <div className="space-y-4">
          {repos.map((repo) => (
            <RepoCard key={repo.name} repo={repo} />
          ))}
        </div>
      )}
    </div>
  );
};

export default Registry;
