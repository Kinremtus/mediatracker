import os
import logging
from dotenv import load_dotenv

# Загружаем .env из корня проекта (на уровень выше backend/)
load_dotenv(os.path.join(os.path.dirname(__file__), "../.env"))

# Настройка логов, чтобы видеть ошибки провайдеров
logging.basicConfig(level=logging.INFO)

from services.email.service import email_service

def test_send():
    recipient = "dubrovskiywarcraft.z@gmail.com"
    username = "TestUser"
    
    print(f"🚀 Отправка тестового письма на {recipient}...")
    
    try:
        result = email_service.send_welcome(
            to=recipient, 
            username=username
        )
        if result:
            print("✅ Письмо успешно отправлено!")
        else:
            print("❌ Все провайдеры вернули ошибку. Проверьте логи выше.")
    except Exception as e:
        print(f"💥 Критическая ошибка при отправке: {e}")

if __name__ == "__main__":
    test_send()
