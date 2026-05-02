import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { register } from "@/api/auth";
import { ModeToggle } from "@/components/mode-toggle";

export default function RegisterPage({
  onSwitchToLogin,
}: {
  onSwitchToLogin: () => void;
}) {
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [success, setSuccess] = useState(false);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (password !== confirm) {
      setError("Пароли не совпадают");
      return;
    }
    setError("");
    setLoading(true);
    try {
      await register(username, email, password);
      setSuccess(true);
    } catch (err) {
      setError((err as Error).message);
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
      {/* Кнопка смены темы */}
      <div className="absolute top-4 right-4 z-20">
        <ModeToggle />
      </div>

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
          <p className="text-white/60 mt-1 text-sm">Создай аккаунт</p>
        </div>

        {success ? (
          <div className="text-center flex flex-col gap-4">
            <p className="text-green-400">Аккаунт создан!</p>
            <Button
              onClick={onSwitchToLogin}
              className="w-full rounded-full"
              style={{ background: "rgba(255,255,255,0.9)", color: "#1a1a2e" }}
            >
              Войти
            </Button>
          </div>
        ) : (
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
              type="email"
              placeholder="Email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
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
            <Input
              type="password"
              placeholder="Повтори пароль"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
              style={{
                background: "rgba(255, 255, 255, 0.08)",
                borderColor: "rgba(255, 255, 255, 0.2)",
              }}
              className="text-white placeholder:text-white/40 h-11 rounded-full px-5"
            />

            {error && (
              <p className="text-red-400 text-sm text-center">{error}</p>
            )}

            <Button
              type="submit"
              disabled={loading}
              className="w-full h-11 mt-1 font-semibold text-sm rounded-full"
              style={{
                background: "rgba(255, 255, 255, 0.9)",
                color: "#1a1a2e",
              }}
            >
              {loading ? "Создаём..." : "Зарегистрироваться"}
            </Button>
          </form>
        )}

        <p className="text-center text-white/50 text-sm">
          Уже есть аккаунт?{" "}
          <button
            onClick={onSwitchToLogin}
            className="text-white/90 font-semibold hover:text-white transition-colors"
          >
            Войти
          </button>
        </p>
      </div>
    </div>
  );
}