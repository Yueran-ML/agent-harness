import configparser


def main():
    config = configparser.ConfigParser()
    config.read("config/settings.ini")
    print("Hello from", config["app"]["name"])


if __name__ == "__main__":
    main()
