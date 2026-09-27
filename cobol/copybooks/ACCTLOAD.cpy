      *================================================================*
      * ACCTLOAD - ACCOUNT CONVERSION FEED RECORD   RECFM=FB LRECL=100 *
      * ONE-TIME LOAD FILE FROM THE LEGACY CORE. DISPLAY NUMERICS.     *
      * USE: COPY ACCTLOAD REPLACING ==:AL:== BY ==XX==.               *
      *================================================================*
       01  :AL:-LOAD-REC.
           05  :AL:-ACCT-ID            PIC X(10).
           05  :AL:-CUST-NAME          PIC X(30).
           05  :AL:-ACCT-TYPE          PIC X(01).
           05  :AL:-STATUS             PIC X(01).
           05  :AL:-OPEN-DATE          PIC 9(08).
           05  :AL:-BALANCE            PIC S9(11)V99
                                       SIGN LEADING SEPARATE.
           05  :AL:-INT-RATE           PIC 9V9(04).
           05  :AL:-OD-LIMIT           PIC 9(07)V99.
           05  FILLER                  PIC X(22).
