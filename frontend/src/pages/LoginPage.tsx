import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { login } from "@/api/auth";

interface LoginPageProps {
  onSwitchToRegister: () => void;
  onLoginSuccess: () => void;
}

export default function LoginPage({
  onSwitchToRegister,
  onLoginSuccess,
}: LoginPageProps) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [rememberMe, setRememberMe] = useState(false);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault();
    setError("");
    setLoading(true);

    try {
      const data = await login(username, password);
      // Сохраняем токен
      if (rememberMe) {
        localStorage.setItem("token", data.access_token);
      } else {
        sessionStorage.setItem("token", data.access_token);
      }
      onLoginSuccess(); // переходим на главную
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div
      className="min-h-screen flex items-center justify-center relative overflow-hidden"
      style={{
        backgroundImage: "url('/river-landscape-illustration-pixel-art-style.jpg')",
        backgroundSize: "cover",
        backgroundPosition: "center",
      }}
    >
      {/* Звёзды */}
      <div className="absolute inset-0 overflow-hidden pointer-events-none">
        {[...Array(80)].map((_, i) => (
          <div
            key={i}
            className="absolute rounded-full bg-white"
            style={{
              width: Math.random() * 2 + 1 + "px",
              height: Math.random() * 2 + 1 + "px",
              top: Math.random() * 100 + "%",
              left: Math.random() * 100 + "%",
              opacity: Math.random() * 0.7 + 0.3,
            }}
          />
        ))}
      </div>

      <div
        className="relative z-10 w-full max-w-md mx-4 rounded-2xl p-8 flex flex-col gap-6"
        style={{
          background: "rgba(255, 255, 255, 0.07)",
          backdropFilter: "blur(24px)",
          WebkitBackdropFilter: "blur(24px)",
          border: "1px solid rgba(255, 255, 255, 0.15)",
          boxShadow: "0 8px 32px rgba(0, 0, 0, 0.4)",
        }}
      >
        <div className="text-center">
          <h1 className="text-3xl font-bold text-white">MediaTracker</h1>
          <p className="text-white/60 mt-1 text-sm">Войди в свой аккаунт</p>
        </div>

        <form onSubmit={handleSubmit} className="flex flex-col gap-3">
          <Input
            placeholder="Имя пользователя"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            style={{
              background: "rgba(255, 255, 255, 0.08)",
              borderColor: "rgba(255, 255, 255, 0.2)",
            }}
            className="text-white placeholder:text-white/40 h-11 rounded-full px-5"
          />
          <Input
            type="password"
            placeholder="Пароль"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            style={{
              background: "rgba(255, 255, 255, 0.08)",
              borderColor: "rgba(255, 255, 255, 0.2)",
            }}
            className="text-white placeholder:text-white/40 h-11 rounded-full px-5"
          />

          {/* Ошибка */}
          {error && (
            <p className="text-red-400 text-sm text-center">{error}</p>
          )}

          <div className="flex items-center gap-2">
            <Checkbox
              id="remember"
              checked={rememberMe}
              onCheckedChange={setRememberMe}
              className="border-white/30 data-[state=checked]:bg-white data-[state=checked]:text-black"
            />
            <Label
              htmlFor="remember"
              className="text-white/70 text-sm cursor-pointer"
            >
              Запомнить меня
            </Label>
          </div>

          <Button
            type="submit"
            disabled={loading}
            className="w-full h-11 mt-1 font-semibold text-sm rounded-full"
            style={{
              background: "rgba(255, 255, 255, 0.9)",
              color: "#1a1a2e",
            }}
          >
            {loading ? "Входим..." : "Войти"}
          </Button>
        </form>

        <p className="text-center text-white/50 text-sm">
          Нет аккаунта?{" "}
          <button
            onClick={onSwitchToRegister}
            className="text-white/90 font-semibold hover:text-white transition-colors"
          >
            Регистрация
          </button>
        </p>
      </div>
    </div>
  );
}