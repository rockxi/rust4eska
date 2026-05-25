import axios from 'axios';

const getBaseURL = () => {
  if (import.meta.env.DEV) {
    return 'http://localhost:8080/api';
  }
  return `http://${window.location.hostname}:8080/api`;
};

const apiClient = axios.create({
  baseURL: getBaseURL(),
  headers: {
    'Content-Type': 'application/json',
  },
});

apiClient.interceptors.request.use(
  (config) => {
    const token = sessionStorage.getItem('r4a_token');
    if (token) {
      config.headers.Authorization = `Bearer ${token}`;
    }
    return config;
  },
  (error) => {
    return Promise.reject(error);
  }
);

export default apiClient;
