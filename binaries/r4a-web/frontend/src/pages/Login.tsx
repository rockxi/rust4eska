import React, { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useAuth } from '../context/AuthContext';
import { KeyRound, Loader2 } from 'lucide-react';

const Login: React.FC = () => {
  const [secret, setSecret] = useState('');
  const [error, setError] = useState('');
  const [isLoading, setIsLoading] = useState(false);
  const { login } = useAuth();
  const navigate = useNavigate();

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!secret.trim()) {
      setError('Cluster secret is required');
      return;
    }

    setIsLoading(true);
    setError('');

    try {
      await login(secret);
      navigate('/');
    } catch {
      setError('Invalid cluster secret or server unavailable');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-[#0b0c10] flex items-center justify-center p-4">
      <div className="w-full max-w-md bg-[#1f2833] rounded-lg shadow-2xl p-8 border border-gray-800">
        <div className="flex flex-col items-center mb-8">
          <div className="w-16 h-16 bg-[#0b0c10] rounded-full flex items-center justify-center mb-4 border border-[#66fcf1]/30">
            <KeyRound className="w-8 h-8 text-[#66fcf1]" />
          </div>
          <h1 className="text-2xl font-bold text-white tracking-wider">r4a cluster</h1>
          <p className="text-gray-400 text-sm mt-2">Enter your cluster secret to continue</p>
        </div>

        <form onSubmit={handleSubmit} className="space-y-6">
          <div>
            <input
              type="password"
              value={secret}
              onChange={(e) => setSecret(e.target.value)}
              placeholder="Cluster Secret"
              className="w-full bg-[#0b0c10] text-white border border-gray-700 rounded px-4 py-3 focus:outline-none focus:border-[#66fcf1] transition-colors placeholder-gray-600"
              disabled={isLoading}
            />
            {error && <p className="text-red-400 text-sm mt-2">{error}</p>}
          </div>

          <button
            type="submit"
            disabled={isLoading}
            className="w-full bg-[#66fcf1] text-[#0b0c10] font-bold py-3 px-4 rounded hover:bg-[#45a29e] transition-colors flex items-center justify-center disabled:opacity-70 disabled:cursor-not-allowed"
          >
            {isLoading ? (
              <Loader2 className="w-5 h-5 animate-spin" />
            ) : (
              'Authenticate'
            )}
          </button>
        </form>
      </div>
    </div>
  );
};

export default Login;
