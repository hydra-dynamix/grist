============================
Maximum complexity
============================

Paragraph with *emphasis*, **strong**, ``literal``, :code:`role`, `link <https://example.invalid/>`_, target_, [1]_, and |product|.

.. _target: section
.. |product| replace:: Grist
.. [1] Located footnote.

.. warning:: inert directive
   :class: fixture

.. unknown-extension:: opaque
   retained without execution

.. code-block:: rust

   fn inert() { /* never executed */ }

+--------+-------+
| Name   | Value |
+========+=======+
| alpha  | 1     |
+--------+-------+

Section
-------

1. ordered
2. retained
