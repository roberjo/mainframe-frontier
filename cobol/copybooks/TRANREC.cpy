      *================================================================*
      * TRANREC  - VALIDATED TRANSACTION RECORD     RECFM=FB LRECL=80  *
      * SIGNED PACKED AMOUNT: CREDITS POSITIVE, DEBITS NEGATIVE.       *
      * USE: COPY TRANREC REPLACING ==:TR:== BY ==XX==.                *
      *================================================================*
       01  :TR:-TRAN-REC.
           05  :TR:-ACCT-ID            PIC X(10).
           05  :TR:-TIMESTAMP.
               10  :TR:-TS-DATE        PIC X(08).
               10  :TR:-TS-TIME        PIC X(06).
           05  :TR:-TXN-ID             PIC X(12).
           05  :TR:-TXN-TYPE           PIC X(02).
               88  :TR:-FEE            VALUE 'FE'.
           05  :TR:-AMOUNT             PIC S9(09)V99   COMP-3.
           05  :TR:-CHANNEL            PIC X(03).
           05  :TR:-DESCRIPTION        PIC X(20).
           05  FILLER                  PIC X(13).
