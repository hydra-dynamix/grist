from base import Base as Parent
__all__ = ["Service"]
# service docs
class Service(Parent):
    """Service documentation."""
    def run(self, value):
        result = helper(value)
        if result:
            return result

def test_service():
    Service().run("ok")
