from fastapi import Depends, HTTPException
from fastapi.security import OAuth2PasswordBearer
from sqlalchemy.orm import Session

import models
import auth
from database import get_db

oauth2_scheme = OAuth2PasswordBearer(tokenUrl="auth/login")


def get_current_user(
    token: str = Depends(oauth2_scheme),
    db: Session = Depends(get_db),
):
    username = auth.decode_token(token)
    if not username:
        raise HTTPException(status_code=401, detail="Неверный токен")
    user = db.query(models.User).filter(
        models.User.username == username
    ).first()
    if not user:
        raise HTTPException(
            status_code=401, detail="Пользователь не найден"
        )
    return user