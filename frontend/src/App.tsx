import { useState } from "react";
import LoginPage from "@/pages/LoginPage";
import RegisterPage from "@/pages/RegisterPage";
import HomePage from "@/pages/HomePage";

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

  if (isLoggedIn) {
    return <HomePage onLogout={handleLogout} />;
  }

  return (
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
  );
}

export default App;