      *================================================================*
      * BUSDATEP - READ AND VALIDATE THE BUSINESS DATE PARM.           *
      * ENDS THE PROGRAM WITH RC=16 IF THE PARM IS MISSING OR INVALID. *
      *================================================================*
       9800-GET-BUS-DATE.
           ACCEPT WS-PARM FROM COMMAND-LINE
           IF WS-PARM(1:8) IS NOT NUMERIC
               DISPLAY 'PARM MUST BE YYYYMMDD, RECEIVED: ' WS-PARM(1:20)
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF
           MOVE WS-PARM(1:8) TO WS-BUS-DATE
           IF FUNCTION TEST-DATE-YYYYMMDD(WS-BUS-DATE) NOT = ZERO
               DISPLAY 'PARM IS NOT A VALID DATE: ' WS-PARM(1:8)
               MOVE 16 TO RETURN-CODE
               STOP RUN
           END-IF.
