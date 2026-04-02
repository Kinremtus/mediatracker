from fastapi import FastAPI
from database import engine, Base
from routers import auth, media, tracking, search

Base.metadata.create_all(bind=engine)

app = FastAPI(title="MediaTracker")

app.include_router(auth.router)
app.include_router(media.router)
app.include_router(tracking.router)
app.include_router(search.router)


@app.get("/health")
async def health():
    return {"status": "ok"}