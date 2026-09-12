.. _my-page:

My RST Page
===========

Intro paragraph with :ref:`a ref <target>` and :term:`Pod` and ``literal``.

.. note::

   A note body.

Section One
-----------

.. tabs::

   .. group-tab:: Helm

      Install with helm.

      .. code-block:: shell-session

         $ helm install cilium

   .. group-tab:: CLI

      Install with cli.

.. toctree::
   :maxdepth: 1

   other/page

Sub A
^^^^^

.. list-table::
   :header-rows: 1

   * - Key
     - Value
   * - a
     - 1

- bullet one
- bullet two

term
   definition text

Section Two
-----------

.. versionadded:: 1.18

   Added feature.

Final :doc:`docs </x>` words.
