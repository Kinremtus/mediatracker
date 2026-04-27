from fastapi import APIRouter, Depends, HTTPException
from fastapi.security import OAuth2PasswordRequestForm
from sqlalchemy.orm import Session

import models
import schemas
import auth
from database import get_db

router = APIRouter(prefix="/auth", tags=["auth"])
# prefix="/auth" — все эндпоинты в этом файле начинаются с /auth
# tags=["auth"] — группировка в Swagger UI


@router.post("/register", response_model=schemas.UserResponse)
def register(user: schemas.UserCreate, db: Session = Depends(get_db)):
    existing = (
        db.query(models.User)
        .filter(models.User.username == user.username)
        .first()
    )
    if existing:
        raise HTTPException(
            status_code=400, detail="Пользователь уже существует"
        )
    db_user = models.User(
        username=user.username,
        email=user.email,
        hashed_password=auth.hash_password(user.password),
    )
    db.add(db_user)
    db.commit()
    db.refresh(db_user)
    return db_user


@router.post("/login", response_model=schemas.Token)
def login(
    form: OAuth2PasswordRequestForm = Depends(),
    db: Session = Depends(get_db),
):
    user = (
        db.query(models.User)
        .filter(models.User.username == form.username)
        .first()
    )
    if not user or not auth.verify_password(
        form.password, user.hashed_password
    ):
        raise HTTPException(
            status_code=401, detail="Неверный логин или пароль"
        )
    token = auth.create_token({"sub": user.username})
    return {"access_token": token, "token_type": "bearer"}