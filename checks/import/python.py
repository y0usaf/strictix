import sys


def main(argv):
    total = 0
    for arg in argv[1:]:
        total += int(arg)
    print(f"total: {total}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
