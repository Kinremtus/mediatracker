export const ProjectSecrets = async () => {
  const maskKeys = [
    "POSTGRES_PASSWORD",
    "POSTGRES_USER",
    "POSTGRES_DB",
    "SECRET_KEY",
    "TELEGRAM_BOT_TOKEN",
    "TMDB_API_KEY",
    "RAWG_API_KEY",
    "MAL_CLIENT_ID",
    "MAL_CLIENT_SECRET",
    "IGDB_CLIENT_ID",
    "IGDB_CLIENT_SECRET",
    "RESEND_API_KEY",
    "MAILERSEND_SMTP_USER",
    "MAILERSEND_SMTP_PASS",
    "EMAIL_FROM",
  ];
  const keyPattern = new RegExp(
    "^(?:" + maskKeys.join("|") + ")=.*",
    "gm"
  );

  return {
    "tool.execute.after": async (input, output) => {
      if (typeof output?.output !== "string") return;
      output.output = output.output
        .replace(keyPattern, (m) => m.split("=")[0] + "=***")
        .replace(/(DATABASE_URL=postgresql:\/\/)[^@]+(@)/g, "$1***$2");
    },
  };
};
