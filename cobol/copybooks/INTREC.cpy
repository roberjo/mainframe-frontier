      *================================================================*
      * INTREC   - INTEREST ACCRUAL JOURNAL RECORD  RECFM=FB LRECL=50  *
      * ONE PER ACCOUNT THAT ACCRUED OR WAS CREDITED INTEREST.         *
      * GDG: FFB.DAILY.INTLOG                                          *
      * USE: COPY INTREC REPLACING ==:IR:== BY ==XX==.                 *
      *================================================================*
       01  :IR:-INT-REC.
           05  :IR:-ACCT-ID            PIC X(10).
           05  :IR:-BUS-DATE           PIC 9(08).
           05  :IR:-BAL-BASIS          PIC S9(11)V99   COMP-3.
           05  :IR:-RATE               PIC 9V9(04)     COMP-3.
           05  :IR:-DAILY-ACCR         PIC S9(07)V9(06) COMP-3.
           05  :IR:-CREDITED           PIC S9(07)V99   COMP-3.
           05  FILLER                  PIC X(10).
