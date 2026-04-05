import { api } from "./client";

export interface TokenResponse {
  access_token: string;
  token_type: string;
}

export interface UserResponse {
  id: number;
  username: string;
}

export async function login(
  username: string,
  password: string
): Promise<TokenResponse> {
  const BASE_URL = import.meta.env.DEV
  ? "https://mediatracker.web-socket-test-bench.site:2053"
  : "";
  const formData = new URLSearchParams();
  formData.append("username", username);
  formData.append("password", password);

  const token =
    localStorage.getItem("token") || sessionStorage.getItem("token");

  const res = await fetch(`${BASE_URL}/auth/login`, {
    method: "POST",
    headers: {
      "Content-Type": "application/x-www-form-urlencoded",
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
    },
    body: formData,
  });

  if (!res.ok) {
    const error = await res.json().catch(() => ({}));
    throw new Error(
      (error as { detail?: string }).detail || "Неверный логин или пароль"
    );
  }

  return res.json() as Promise<TokenResponse>;
}

export async function register(
  username: string,
  password: string
): Promise<UserResponse> {
  return api.post<UserResponse>("/auth/register", { username, password });
}