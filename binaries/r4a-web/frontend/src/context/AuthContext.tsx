import React, { createContext, useContext, useState } from 'react';
import type { ReactNode } from 'react';
import apiClient from '../api/client';

interface User {
  id: string;
  username: string;
  created_at: string;
}

interface AuthContextType {
  token: string | null;
  user: User | null;
  isAuthenticated: boolean;
  login: (secret: string) => Promise<void>;
  logout: () => void;
}

const AuthContext = createContext<AuthContextType | undefined>(undefined);

export const AuthProvider: React.FC<{ children: ReactNode }> = ({ children }) => {
  const [token, setToken] = useState<string | null>(() => sessionStorage.getItem('r4a_token'));
  const [user, setUser] = useState<User | null>(() => {
    const storedUser = sessionStorage.getItem('r4a_user');
    return storedUser ? JSON.parse(storedUser) : null;
  });

  const isAuthenticated = !!token;

  const login = async (secret: string) => {
    try {
      const response = await apiClient.post('/tokens/exchange', null, {
        headers: {
          'X-R4A-Secret': secret,
        },
      });
      
      const { id, username, created_at } = response.data;
      
      setToken(id);
      setUser({ id, username, created_at });
      
      sessionStorage.setItem('r4a_token', id);
      sessionStorage.setItem('r4a_user', JSON.stringify({ id, username, created_at }));
    } catch (error) {
      console.error('Login failed:', error);
      throw error;
    }
  };

  const logout = () => {
    setToken(null);
    setUser(null);
    sessionStorage.removeItem('r4a_token');
    sessionStorage.removeItem('r4a_user');
  };

  return (
    <AuthContext.Provider value={{ token, user, isAuthenticated, login, logout }}>
      {children}
    </AuthContext.Provider>
  );
};

// eslint-disable-next-line react-refresh/only-export-components
export const useAuth = () => {
  const context = useContext(AuthContext);
  if (context === undefined) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
};
