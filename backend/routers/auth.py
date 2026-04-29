from services.email.service import email_service
from fastapi import APIRouter, Depends, HTTPException, status
from fastapi.security import OAuth2PasswordRequestForm, OAuth2PasswordBearer
from sqlalchemy.orm import Session

import models
import schemas
import auth
from database import get_db

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="auth/login")

def get_current_user(token: str = Depends(oauth2_scheme), db: Session = Depends(get_db)):
    username = auth.decode_token(token)
    if not username:
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Невалидный токен или сессия истекла",
        )
    user = db.query(models.User).filter(models.User.username == username).first()
    if not user:
        raise HTTPException(
            status_code=status.HTTP_404_NOT_FOUND,
            detail="Пользователь не найден",
        )
    return user

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
    email_service.send_welcome(db_user.email, db_user.username)
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


@router.delete("/me", status_code=status.HTTP_204_NO_CONTENT)
def delete_account(
    current_user: models.User = Depends(get_current_user), 
    db: Session = Depends(get_db)
):
    db.delete(current_user)
    db.commit()
    return None
