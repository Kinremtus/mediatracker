import { useState } from "react";
import LoginPage from "@/pages/LoginPage";
import RegisterPage from "@/pages/RegisterPage";
import HomePage from "@/pages/HomePage";
import { ThemeProvider } from "@/components/theme-provider";

function App() {
  const [page, setPage] = useState("login");
  const [isLoggedIn, setIsLoggedIn] = useState(
    !!(localStorage.getItem("token") || sessionStorage.getItem("token"))
  );

  function handleLogout() {
    localStorage.removeItem("token");
    sessionStorage.removeItem("token");
    setIsLoggedIn(false);
  }

  return (
    <ThemeProvider defaultTheme="graphite" storageKey="vite-ui-theme" themes={["light", "graphite"]} attribute="class" enableSystem={false} enableColorScheme={false}>
      {isLoggedIn ? (
        <HomePage onLogout={handleLogout} />
      ) : (
        <>
          {page === "login" && (
            <LoginPage
              onSwitchToRegister={() => setPage("register")}
              onLoginSuccess={() => setIsLoggedIn(true)}
            />
          )}
          {page === "register" && (
            <RegisterPage onSwitchToLogin={() => setPage("login")} />
          )}
        </>
      )}
    </ThemeProvider>
  );
}

export default App;