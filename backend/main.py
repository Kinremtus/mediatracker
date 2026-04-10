from fastapi import FastAPI
from database import engine, Base
from routers import auth, media, tracking, search
from fastapi.middleware.cors import CORSMiddleware

Base.metadata.create_all(bind=engine)

app = FastAPI(title="MediaTracker", redirect_slashes=False)

app.add_middleware(
    CORSMiddleware,
    allow_origins=[
    "http://localhost:5173",
    "https://mediatracker.web-socket-test-bench.site:2053",
    "https://mediatracker.web-socket-test-bench.site",  # ← добавь без порта тоже
    ],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

app.include_router(auth.router)
app.include_router(media.router)
app.include_router(tracking.router)
app.include_router(search.router)


@app.get("/health")
async def health():
    return {"status": "ok"}